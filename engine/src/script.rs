use std::fmt;

use glam::Vec2;
use serde::Deserialize;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

use crate::input::InputEvent;

/// Событие сценария и тик, на котором оно применяется.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptEvent {
    /// Тик `1..=ticks`.
    pub tick: u32,
    /// Событие ввода.
    pub event: InputEvent,
}

/// Разобранный и проверенный сценарий runner.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    /// Сид, попадающий в `ctx.seed` до `Game::init`.
    pub seed: u64,
    /// Сколько раз вызвать `update`.
    pub ticks: u32,
    /// Размер в физических пикселях; `None` — размер из `Config`.
    pub size: Option<(u32, u32)>,
    /// Физических пикселей на точку egui.
    pub scale: f32,
    /// Тики снимков по возрастанию без повторов, последний — `ticks`.
    pub snapshots: Vec<u32>,
    events: Vec<ScriptEvent>,
}

/// Ошибка разбора сценария.
#[derive(Debug, PartialEq)]
pub enum ScriptError {
    /// Не TOML или неверная форма верхнего уровня.
    Toml(String),
    /// Значение вне допустимого.
    Invalid(String),
    /// Ошибка записи `event[index]` (с нуля).
    Event {
        /// Номер записи.
        index: usize,
        /// Причина.
        reason: String,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(e) | Self::Invalid(e) => write!(f, "сценарий: {e}"),
            Self::Event { index, reason } => write!(f, "сценарий: event[{index}]: {reason}"),
        }
    }
}

impl std::error::Error for ScriptError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScript {
    #[serde(default)]
    seed: u64,
    ticks: u32,
    size: Option<[u32; 2]>,
    #[serde(default = "one")]
    scale: f32,
    #[serde(default)]
    snapshots: Vec<u32>,
    #[serde(default)]
    event: Vec<toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvent {
    tick: u32,
    press: Option<KeyCode>,
    release: Option<KeyCode>,
    cursor: Option<[f32; 2]>,
    cursor_left: Option<bool>,
    button_press: Option<MouseButton>,
    button_release: Option<MouseButton>,
    motion: Option<[f32; 2]>,
    wheel: Option<[f32; 2]>,
    focus: Option<bool>,
}

fn one() -> f32 {
    1.0
}

fn invalid<T>(reason: impl Into<String>) -> Result<T, ScriptError> {
    Err(ScriptError::Invalid(reason.into()))
}

impl RawEvent {
    fn into_event(self) -> Result<InputEvent, String> {
        for (name, v) in [
            ("cursor", self.cursor),
            ("motion", self.motion),
            ("wheel", self.wheel),
        ] {
            if v.is_some_and(|[x, y]| !(x.is_finite() && y.is_finite())) {
                return Err(format!("{name}: нужны конечные числа"));
            }
        }
        let key = |code, state| InputEvent::Key {
            code,
            state,
            repeat: false,
        };
        let mut found = Vec::with_capacity(1);
        if let Some(c) = self.press {
            found.push(key(c, ElementState::Pressed));
        }
        if let Some(c) = self.release {
            found.push(key(c, ElementState::Released));
        }
        if let Some([x, y]) = self.cursor {
            found.push(InputEvent::Cursor(Some(Vec2::new(x, y))));
        }
        match self.cursor_left {
            Some(true) => found.push(InputEvent::Cursor(None)),
            Some(false) => return Err("cursor_left = false не имеет смысла".into()),
            None => {}
        }
        if let Some(button) = self.button_press {
            found.push(InputEvent::Button {
                button,
                state: ElementState::Pressed,
            });
        }
        if let Some(button) = self.button_release {
            found.push(InputEvent::Button {
                button,
                state: ElementState::Released,
            });
        }
        if let Some([x, y]) = self.motion {
            found.push(InputEvent::Motion(Vec2::new(x, y)));
        }
        if let Some([x, y]) = self.wheel {
            found.push(InputEvent::Wheel(Vec2::new(x, y)));
        }
        if let Some(on) = self.focus {
            found.push(InputEvent::Focus(on));
        }
        match found.as_slice() {
            [one] => Ok(*one),
            [] => Err("нет действия".into()),
            _ => Err("больше одного действия".into()),
        }
    }
}

/// Нажатие и отпускание одной клавиши или кнопки: `update` видел бы только итог.
fn opposite(first: InputEvent, second: InputEvent) -> bool {
    match (first, second) {
        (
            InputEvent::Key {
                code: a, state: sa, ..
            },
            InputEvent::Key {
                code: b, state: sb, ..
            },
        ) => a == b && sa != sb,
        (
            InputEvent::Button {
                button: a,
                state: sa,
            },
            InputEvent::Button {
                button: b,
                state: sb,
            },
        ) => a == b && sa != sb,
        _ => false,
    }
}

impl Script {
    /// Разбирает и проверяет сценарий.
    ///
    /// # Errors
    /// Ошибка TOML, неизвестное поле, значение вне диапазона; ошибка записи называет её номер.
    pub fn parse(text: &str) -> Result<Self, ScriptError> {
        let raw: RawScript = toml::from_str(text).map_err(|e| ScriptError::Toml(e.to_string()))?;
        if raw.ticks == 0 {
            return invalid("ticks = 0");
        }
        if let Some([w, h]) = raw.size
            && (w == 0 || h == 0)
        {
            return invalid(format!("size = [{w}, {h}]: нулевая сторона"));
        }
        if !(raw.scale.is_finite() && raw.scale > 0.0) {
            return invalid(format!("scale = {}: нужно число больше нуля", raw.scale));
        }
        if let Some(&s) = raw.snapshots.iter().find(|&&s| s > raw.ticks) {
            return invalid(format!("снимок {s} вне 0..={}", raw.ticks));
        }
        let mut events: Vec<ScriptEvent> = Vec::with_capacity(raw.event.len());
        for (index, value) in raw.event.into_iter().enumerate() {
            let fail = |reason: String| ScriptError::Event { index, reason };
            let ev: RawEvent = value.try_into().map_err(|e| fail(e.to_string()))?;
            let tick = ev.tick;
            if tick == 0 || tick > raw.ticks {
                return Err(fail(format!("tick = {tick} вне 1..={}", raw.ticks)));
            }
            if events.last().is_some_and(|p| p.tick > tick) {
                return Err(fail(format!("tick = {tick} меньше предыдущего")));
            }
            let event = ev.into_event().map_err(fail)?;
            let same_tick = events.iter().rev().take_while(|p| p.tick == tick);
            if same_tick.into_iter().any(|p| opposite(p.event, event)) {
                return Err(fail(
                    "нажатие и отпускание одной клавиши или кнопки в одном тике: нажатие потеряется".into(),
                ));
            }
            events.push(ScriptEvent { tick, event });
        }
        let mut snapshots = raw.snapshots;
        snapshots.push(raw.ticks);
        snapshots.sort_unstable();
        snapshots.dedup();
        Ok(Self {
            seed: raw.seed,
            ticks: raw.ticks,
            size: raw.size.map(|[w, h]| (w, h)),
            scale: raw.scale,
            snapshots,
            events,
        })
    }

    /// События тика в порядке файла.
    #[must_use]
    pub fn events_at(&self, tick: u32) -> &[ScriptEvent] {
        let from = self.events.partition_point(|e| e.tick < tick);
        let len = self.events[from..].partition_point(|e| e.tick == tick);
        &self.events[from..from + len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
seed = 7
ticks = 600
size = [800, 600]
scale = 2.0
snapshots = [120, 0, 120]

[[event]]
tick = 1
press = "KeyW"

[[event]]
tick = 30
cursor = [400.0, 300.0]

[[event]]
tick = 30
button_press = "Left"

[[event]]
tick = 31
motion = [3.0, -1.0]

[[event]]
tick = 40
wheel = [0.0, 1.0]

[[event]]
tick = 50
focus = false

[[event]]
tick = 60
cursor_left = true

[[event]]
tick = 61
release = "KeyW"

[[event]]
tick = 62
button_release = "Left"
"#;

    fn event_err(body: &str) -> (usize, String) {
        let text = format!("ticks = 10\n{body}");
        match Script::parse(&text) {
            Err(ScriptError::Event { index, reason }) => (index, reason),
            other => panic!("ожидалась ошибка записи, вышло {other:?}"),
        }
    }

    fn invalid_err(text: &str) -> String {
        match Script::parse(text) {
            Err(ScriptError::Invalid(r)) => r,
            other => panic!("ожидалась Invalid, вышло {other:?}"),
        }
    }

    #[test]
    fn parses_full_example() {
        let s = Script::parse(FULL).unwrap();
        assert_eq!(
            (s.seed, s.ticks, s.size, s.scale),
            (7, 600, Some((800, 600)), 2.0)
        );
        assert_eq!(s.snapshots, [0, 120, 600]);
        assert_eq!(
            s.events_at(1),
            [ScriptEvent {
                tick: 1,
                event: InputEvent::Key {
                    code: KeyCode::KeyW,
                    state: ElementState::Pressed,
                    repeat: false
                }
            }]
        );
        let at30: Vec<_> = s.events_at(30).iter().map(|e| e.event).collect();
        assert_eq!(
            at30,
            [
                InputEvent::Cursor(Some(Vec2::new(400.0, 300.0))),
                InputEvent::Button {
                    button: MouseButton::Left,
                    state: ElementState::Pressed
                }
            ]
        );
        assert_eq!(
            s.events_at(31)[0].event,
            InputEvent::Motion(Vec2::new(3.0, -1.0))
        );
        assert_eq!(
            s.events_at(40)[0].event,
            InputEvent::Wheel(Vec2::new(0.0, 1.0))
        );
        assert_eq!(s.events_at(50)[0].event, InputEvent::Focus(false));
        assert_eq!(s.events_at(60)[0].event, InputEvent::Cursor(None));
        assert_eq!(
            s.events_at(61)[0].event,
            InputEvent::Key {
                code: KeyCode::KeyW,
                state: ElementState::Released,
                repeat: false
            }
        );
        assert_eq!(
            s.events_at(62)[0].event,
            InputEvent::Button {
                button: MouseButton::Left,
                state: ElementState::Released
            }
        );
        assert_eq!(s.events_at(2), []);
        assert_eq!(s.events_at(0), []);
        assert_eq!(s.events_at(601), []);
    }

    #[test]
    fn defaults() {
        let s = Script::parse("ticks = 5").unwrap();
        assert_eq!((s.seed, s.size, s.scale), (0, None, 1.0));
        assert_eq!(s.snapshots, [5]);
    }

    #[test]
    fn last_tick_always_snapshotted_sorted_unique() {
        let s = Script::parse("ticks = 9\nsnapshots = [9, 3, 3, 0]").unwrap();
        assert_eq!(s.snapshots, [0, 3, 9]);
    }

    #[test]
    fn event_errors_name_the_record() {
        let (i, r) =
            event_err("[[event]]\ntick = 1\npress = \"KeyW\"\n[[event]]\ntick = 2\nbogus = 1");
        assert_eq!(i, 1);
        assert!(r.contains("bogus"), "{r}");
        let (i, r) = event_err("[[event]]\ntick = 1\npress = \"KeyW\"\nfocus = true");
        assert_eq!((i, r.as_str()), (0, "больше одного действия"));
        let (i, r) = event_err("[[event]]\ntick = 1");
        assert_eq!((i, r.as_str()), (0, "нет действия"));
        let (i, r) = event_err("[[event]]\ntick = 1\npress = \"NoSuchKey\"");
        assert_eq!(i, 0);
        assert!(r.contains("NoSuchKey"), "{r}");
        let (i, r) = event_err("[[event]]\ntick = 0\nfocus = true");
        assert_eq!(i, 0);
        assert!(r.contains("tick = 0"), "{r}");
        let (i, r) =
            event_err("[[event]]\ntick = 1\nfocus = true\n[[event]]\ntick = 11\nfocus = true");
        assert_eq!(i, 1);
        assert!(r.contains("tick = 11"), "{r}");
        let (i, r) =
            event_err("[[event]]\ntick = 5\nfocus = true\n[[event]]\ntick = 4\nfocus = true");
        assert_eq!(i, 1);
        assert!(r.contains("меньше предыдущего"), "{r}");
        let (i, _) = event_err("[[event]]\ntick = 1\ncursor_left = false");
        assert_eq!(i, 0);
        let (i, _) = event_err("[[event]]\nfocus = true");
        assert_eq!(i, 0);
    }

    #[test]
    fn non_finite_event_values_name_the_record() {
        for field in ["cursor", "motion", "wheel"] {
            for bad in ["nan", "inf", "-inf"] {
                let (i, r) = event_err(&format!(
                    "[[event]]\ntick = 1\nfocus = true\n[[event]]\ntick = 2\n{field} = [1.0, {bad}]"
                ));
                assert_eq!(i, 1);
                assert!(r.contains(field), "{r}");
            }
        }
    }

    #[test]
    fn press_and_release_in_one_tick_is_error() {
        let (i, r) = event_err(
            "[[event]]\ntick = 3\npress = \"KeyW\"\n[[event]]\ntick = 3\nrelease = \"KeyW\"",
        );
        assert_eq!(i, 1);
        assert!(r.contains("одном тике"), "{r}");
        let (i, _) = event_err(
            "[[event]]\ntick = 3\nbutton_release = \"Left\"\n[[event]]\ntick = 3\nbutton_press = \"Left\"",
        );
        assert_eq!(i, 1);
        Script::parse(
            "ticks = 4\n[[event]]\ntick = 3\npress = \"KeyW\"\n[[event]]\ntick = 3\nrelease = \"KeyA\"\n[[event]]\ntick = 4\nrelease = \"KeyW\"",
        )
        .unwrap();
    }

    #[test]
    fn seed_takes_full_u64() {
        let seed = |text: &str| Script::parse(text).map(|s| s.seed);
        assert_eq!(seed("ticks = 1\nseed = 18446744073709551615"), Ok(u64::MAX));
        assert!(seed("ticks = 1\nseed = 18446744073709551616").is_err());
    }

    #[test]
    fn bad_tick_and_snapshot_types_are_errors() {
        assert!(Script::parse("ticks = 3\n[[event]]\ntick = -1\nfocus = true").is_err());
        assert!(Script::parse("ticks = 3\n[[event]]\ntick = 1.5\nfocus = true").is_err());
        assert!(Script::parse("ticks = 3\nsnapshots = [\"a\"]").is_err());
        assert!(Script::parse("ticks = 3\n[[event]]").is_err());
    }

    #[test]
    fn equal_ticks_allowed() {
        Script::parse(
            "ticks = 2\n[[event]]\ntick = 2\nfocus = true\n[[event]]\ntick = 2\nfocus = false",
        )
        .unwrap();
    }

    #[test]
    fn header_errors() {
        assert!(invalid_err("ticks = 0").contains("ticks"));
        assert!(invalid_err("ticks = 1\nsize = [0, 5]").contains("size"));
        assert!(invalid_err("ticks = 1\nsize = [5, 0]").contains("size"));
        assert!(invalid_err("ticks = 1\nscale = 0.0").contains("scale"));
        assert!(invalid_err("ticks = 1\nscale = -1.0").contains("scale"));
        assert!(invalid_err("ticks = 1\nscale = nan").contains("scale"));
        assert!(invalid_err("ticks = 3\nsnapshots = [4]").contains("снимок 4"));
        assert!(matches!(
            Script::parse("ticks = 3\nbogus = 1"),
            Err(ScriptError::Toml(_))
        ));
        assert!(matches!(
            Script::parse("seed = 1"),
            Err(ScriptError::Toml(_))
        ));
        assert!(matches!(Script::parse("= ="), Err(ScriptError::Toml(_))));
    }

    #[test]
    fn snapshot_at_ticks_is_allowed() {
        let s = Script::parse("ticks = 3\nsnapshots = [3]").unwrap();
        assert_eq!(s.snapshots, [3]);
    }

    #[test]
    fn error_display_names_record() {
        let e = ScriptError::Event {
            index: 2,
            reason: "x".into(),
        };
        assert_eq!(e.to_string(), "сценарий: event[2]: x");
        assert_eq!(ScriptError::Invalid("y".into()).to_string(), "сценарий: y");
        assert_eq!(ScriptError::Toml("z".into()).to_string(), "сценарий: z");
    }
}
