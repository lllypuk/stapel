use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::ui::{to_input, wants_pointer};
use crate::{
    CANVAS_FORMAT, Canvas, Config, Context, FixedClock, FrameRenderer, Game, Gpu, InputEvent,
    MouseButton, Script, ScriptError, Vec2, VirtualGrab, step_visual,
};

/// Снимок кадра в отчёте.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Snapshot {
    /// Тик снимка.
    pub tick: u32,
    /// Имя PNG внутри каталога вывода; `None` без каталога.
    pub png: Option<String>,
    /// Спрайтов в кадре.
    pub sprites: usize,
    /// Строки `draw_text`, отправленные в кадр, а не видимые: перекрытый HUD тут не виден.
    pub texts: Vec<String>,
    /// `Game::report` на этом тике.
    pub report: serde_json::Value,
}

/// Итог прогона сценария.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunReport {
    /// Сколько тиков выполнено.
    pub ticks: u32,
    /// Имя адаптера.
    pub adapter: String,
    /// Графический API адаптера.
    pub backend: String,
    /// Снимки по возрастанию тика.
    pub snapshots: Vec<Snapshot>,
}

/// Ошибка запуска или прогона.
pub enum RunError {
    /// Не создался или упал цикл событий окна.
    EventLoop(winit::error::EventLoopError),
    /// Неверные аргументы командной строки.
    Args(String),
    /// Сценарий не разобрался.
    Script(ScriptError),
    /// Нет адаптера для офскрин-рендера.
    NoAdapter(String),
    /// Размер сценария больше лимита текстур устройства.
    Size {
        /// Размер сценария.
        size: (u32, u32),
        /// Предел стороны.
        max: u32,
    },
    /// `Game::report` вернул не JSON-объект.
    Report {
        /// Тик снимка.
        tick: u32,
    },
    /// Окно закрыли раньше последнего тика.
    Closed {
        /// Последний выполненный тик.
        tick: u32,
    },
    /// Нет прогресса дольше `STALL_TIMEOUT`.
    Stalled {
        /// Последний выполненный тик.
        tick: u32,
    },
    /// Ошибка файловой системы.
    Io(std::io::Error),
    /// Ошибка записи PNG.
    Png(image::ImageError),
}

/// Сколько окно режима `--window` может не просить кадров, прежде чем прогон кончится ошибкой.
pub(crate) const STALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Окно простояло без кадров слишком долго.
pub(crate) fn stalled(idle: Duration) -> bool {
    idle >= STALL_TIMEOUT
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(e) => write!(f, "цикл событий: {e}"),
            Self::Args(e) => write!(f, "аргументы: {e}"),
            Self::Script(e) => e.fmt(f),
            Self::NoAdapter(e) => write!(f, "нет адаптера GPU: {e}"),
            Self::Size { size, max } => write!(
                f,
                "размер сценария {}x{} больше лимита текстуры {max}",
                size.0, size.1
            ),
            Self::Report { tick } => write!(f, "тик {tick}: Game::report вернул не объект"),
            Self::Closed { tick } => write!(f, "окно закрыто на тике {tick}, сценарий не кончился"),
            Self::Stalled { tick } => write!(
                f,
                "нет прогресса {} с на тике {tick}; причина не установлена, можно попробовать другой дисплей (env -u WAYLAND_DISPLAY)",
                STALL_TIMEOUT.as_secs()
            ),
            Self::Io(e) => write!(f, "файл: {e}"),
            Self::Png(e) => write!(f, "PNG: {e}"),
        }
    }
}

/// `main` возвращает `Result`, и без этого пользователь видит `Stalled { tick: 0 }` вместо причины.
impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for RunError {}

impl From<winit::error::EventLoopError> for RunError {
    fn from(e: winit::error::EventLoopError) -> Self {
        Self::EventLoop(e)
    }
}

impl From<ScriptError> for RunError {
    fn from(e: ScriptError) -> Self {
        Self::Script(e)
    }
}

impl From<wgpu::RequestAdapterError> for RunError {
    fn from(e: wgpu::RequestAdapterError) -> Self {
        Self::NoAdapter(e.to_string())
    }
}

impl From<std::io::Error> for RunError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<image::ImageError> for RunError {
    fn from(e: image::ImageError) -> Self {
        Self::Png(e)
    }
}

/// Проигрывает сценарий без окна: по виртуальному кадру на тик, кадр рисуется в `Canvas`.
pub struct Runner<G: Game> {
    gpu: Gpu,
    game: G,
    ctx: Context,
    clock: FixedClock,
    egui: egui::Context,
    canvas: Canvas,
    frame: FrameRenderer,
    script: Script,
    size: (u32, u32),
    tick: Option<u32>,
    snapshots: Vec<Snapshot>,
}

impl<G: Game> Runner<G> {
    /// Создаёт игру с сидом сценария; кадров ещё нет.
    ///
    /// # Errors
    /// Размер сценария больше лимита текстуры устройства.
    pub fn new(gpu: &Gpu, config: &Config, script: &Script) -> Result<Self, RunError> {
        let size = script.size.unwrap_or(config.size);
        let max = gpu.device.limits().max_texture_dimension_2d;
        if size.0.max(size.1) > max {
            return Err(RunError::Size { size, max });
        }
        let clock = FixedClock::new(config.tick_hz, Duration::MAX);
        let mut ctx = Context::new(clock.step());
        ctx.focused = true;
        ctx.seed = script.seed;
        ctx.gfx.record_text = true;
        let game = G::init(&mut ctx);
        let canvas = Canvas::new(gpu, size);
        let frame = FrameRenderer::new(
            gpu,
            CANVAS_FORMAT,
            canvas.ui_format(),
            size,
            config.pixel_perfect,
        );
        Ok(Self {
            gpu: gpu.clone(),
            game,
            ctx,
            clock,
            egui: egui::Context::default(),
            canvas,
            frame,
            script: script.clone(),
            size,
            tick: None,
            snapshots: Vec::new(),
        })
    }

    /// Выполняет следующий тик, начиная с нулевого; `None` — сценарий кончился.
    pub fn step(&mut self) -> Option<u32> {
        let tick = match self.tick {
            None => 0,
            Some(t) if t < self.script.ticks => t + 1,
            Some(_) => return None,
        };
        self.tick = Some(tick);
        let step = self.clock.step();
        let mut events = Vec::new();
        for scripted in self.script.events_at(tick) {
            let event = scripted.event;
            let cursor = self.ctx.input.cursor();
            events.extend(egui_event(&event, cursor, self.script.scale));
            if to_input(&event, wants_pointer(&self.egui, cursor)) {
                self.ctx.apply(event);
            }
        }
        let mut raw = raw_input(self.size, self.script.scale, tick, step, self.ctx.focused);
        raw.events = events;
        let dt = if tick == 0 { Duration::ZERO } else { step };
        let mut frame = step_visual(
            &mut self.game,
            &mut self.ctx,
            &mut self.clock,
            &self.egui,
            raw,
            dt,
            &mut VirtualGrab,
        );
        self.frame.upload(&mut self.ctx.assets, &frame.ui);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let (shown, mut buffers) = self.frame.draw(
            &mut encoder,
            self.canvas.view(),
            self.canvas.ui_view(),
            &self.ctx.gfx,
            &self.ctx.assets,
            &frame.ui,
        );
        buffers.push(encoder.finish());
        self.gpu.queue.submit(buffers);
        self.frame.release(&mut frame.ui);
        self.ctx.shown = Some(shown);
        Some(tick)
    }

    /// Снимает последний выполненный тик; с `out` пишет PNG.
    ///
    /// # Errors
    /// `report` не объект; ошибка записи PNG.
    ///
    /// # Panics
    /// До первого `step`.
    pub fn snapshot(&mut self, out: Option<&Path>) -> Result<(), RunError> {
        let tick = self.tick.expect("snapshot before the first step");
        let report = self.game.report();
        if !report.is_object() {
            return Err(RunError::Report { tick });
        }
        let png = out
            .map(|dir| -> Result<String, RunError> {
                fs::create_dir_all(dir)?;
                let name = format!("tick-{tick:06}.png");
                image::save_buffer(
                    dir.join(&name),
                    &self.canvas.read_rgba(&self.gpu),
                    self.size.0,
                    self.size.1,
                    image::ColorType::Rgba8,
                )?;
                Ok(name)
            })
            .transpose()?;
        self.snapshots.push(Snapshot {
            tick,
            png,
            sprites: self.ctx.gfx.sprites.len(),
            texts: self.ctx.gfx.texts.clone(),
            report,
        });
        Ok(())
    }

    /// Итог прогона.
    #[must_use]
    pub fn finish(self) -> RunReport {
        let info = self.gpu.adapter.get_info();
        RunReport {
            ticks: self.tick.unwrap_or(0),
            adapter: info.name,
            backend: format!("{:?}", info.backend),
            snapshots: self.snapshots,
        }
    }

    /// Кадр, в который рисует `step`.
    #[must_use]
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Последний выполненный тик; до первого `step` — 0.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick.unwrap_or(0)
    }

    /// `step` и снимок, если тик в списке сценария; `None` — сценарий кончился.
    ///
    /// # Errors
    /// Ошибки `snapshot`.
    pub fn advance(&mut self, out: Option<&Path>) -> Result<Option<u32>, RunError> {
        let Some(tick) = self.step() else {
            return Ok(None);
        };
        if self.script.snapshots.binary_search(&tick).is_ok() {
            self.snapshot(out)?;
        }
        Ok(Some(tick))
    }

    fn drive(&mut self, out: Option<&Path>) -> Result<(), RunError> {
        for _ in 0..=self.script.ticks {
            self.advance(out)?;
        }
        Ok(())
    }
}

/// Прогоняет сценарий на офскрин-устройстве; с `out` пишет PNG снимков.
///
/// # Errors
/// Нет адаптера, размер больше лимита, `report` не объект, ошибка записи.
pub fn run_script<G: Game>(
    config: &Config,
    script: &Script,
    out: Option<&Path>,
) -> Result<RunReport, RunError> {
    let gpu = Gpu::headless()?;
    let mut runner = Runner::<G>::new(&gpu, config, script)?;
    runner.drive(out)?;
    Ok(runner.finish())
}

fn raw_input(
    size: (u32, u32),
    scale: f32,
    tick: u32,
    step: Duration,
    focused: bool,
) -> egui::RawInput {
    #[expect(clippy::cast_precision_loss, reason = "размер окна — тысячи")]
    let points = egui::vec2(size.0 as f32, size.1 as f32) / scale;
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
        time: Some(f64::from(tick) * step.as_secs_f64()),
        predicted_dt: step.as_secs_f32(),
        focused,
        ..Default::default()
    };
    raw.viewports.insert(
        raw.viewport_id,
        egui::ViewportInfo {
            native_pixels_per_point: Some(scale),
            ..Default::default()
        },
    );
    raw
}

fn egui_event(event: &InputEvent, cursor: Option<Vec2>, scale: f32) -> Option<egui::Event> {
    let point = |p: Vec2| egui::pos2(p.x / scale, p.y / scale);
    Some(match *event {
        InputEvent::Key {
            code,
            state,
            repeat,
        } => egui::Event::Key {
            key: egui_key(code)?,
            physical_key: None,
            pressed: state.is_pressed(),
            repeat,
            modifiers: egui::Modifiers::NONE,
        },
        InputEvent::Button { button, state } => egui::Event::PointerButton {
            pos: point(cursor?),
            button: egui_button(button)?,
            pressed: state.is_pressed(),
            modifiers: egui::Modifiers::NONE,
        },
        InputEvent::Cursor(Some(p)) => egui::Event::PointerMoved(point(p)),
        InputEvent::Cursor(None) => egui::Event::PointerGone,
        InputEvent::Motion(d) => egui::Event::MouseMoved(egui::vec2(d.x, d.y) / scale),
        InputEvent::Wheel(lines) => egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(lines.x, lines.y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
        InputEvent::Focus(focused) => egui::Event::WindowFocused(focused),
    })
}

fn egui_button(button: MouseButton) -> Option<egui::PointerButton> {
    Some(match button {
        MouseButton::Left => egui::PointerButton::Primary,
        MouseButton::Right => egui::PointerButton::Secondary,
        MouseButton::Middle => egui::PointerButton::Middle,
        MouseButton::Back => egui::PointerButton::Extra1,
        MouseButton::Forward => egui::PointerButton::Extra2,
        MouseButton::Other(_) => return None,
    })
}

fn egui_key(code: crate::KeyCode) -> Option<egui::Key> {
    let name = format!("{code:?}");
    let name = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        .unwrap_or(&name);
    egui::Key::from_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::gpu_full;
    use crate::{Camera2D, Camera3D, Color, Font, Handle, KeyCode, Scene, vec2, vec3};
    use winit::event::ElementState::{Pressed, Released};

    const FONT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/fonts/JetBrainsMono-Regular.ttf"
    );
    const STEP: Duration = Duration::from_millis(10);

    fn config(tick_hz: u32) -> Config {
        Config {
            title: String::new(),
            size: (64, 64),
            tick_hz,
            vsync: false,
            pixel_perfect: false,
        }
    }

    struct Toy {
        init_focused: bool,
        updates: u32,
        draws: u32,
        presses: u32,
        clicked: u32,
        downs: Vec<bool>,
        worlds: Vec<Option<f32>>,
        rays: Vec<Option<f32>>,
        deltas: Vec<Vec2>,
        grab: bool,
        font: Handle<Font>,
    }

    impl Game for Toy {
        fn init(ctx: &mut Context) -> Self {
            Self {
                init_focused: ctx.focused,
                font: ctx.assets.load_font("missing.ttf", 16.0),
                updates: 0,
                draws: 0,
                presses: 0,
                clicked: 0,
                downs: Vec::new(),
                worlds: Vec::new(),
                rays: Vec::new(),
                deltas: Vec::new(),
                grab: false,
            }
        }

        fn update(&mut self, ctx: &mut Context) {
            self.updates += 1;
            self.presses += u32::from(ctx.input.mouse_just_pressed(MouseButton::Left));
            self.downs.push(ctx.input.mouse_down(MouseButton::Left));
            self.worlds.push(ctx.cursor_world().map(|p| p.x));
            self.rays.push(ctx.cursor_ray().map(|r| r.origin.x));
            self.deltas.push(ctx.input.mouse_delta());
            ctx.cursor_grab = self.grab;
        }

        fn draw(&mut self, ctx: &mut Context, _: f32) {
            self.draws += 1;
            #[expect(clippy::cast_precision_loss, reason = "тиков единицы")]
            let x = self.updates as f32;
            ctx.gfx.camera = Camera2D {
                center: vec2(x, 0.0),
                size: Vec2::splat(64.0),
            };
            ctx.gfx.scene = Some(Scene {
                camera: Camera3D {
                    pos: vec3(x, 0.0, 0.0),
                    ..Default::default()
                },
                ..Default::default()
            });
            ctx.gfx.draw_text(
                &ctx.assets,
                self.font,
                "hud",
                Vec2::ZERO,
                1.0,
                Color::WHITE,
                0,
            );
        }

        fn ui(&mut self, ui: &egui::Context) {
            crate::ui::set_font(ui, FONT);
            egui::Window::new("w")
                .fixed_pos([10.0, 10.0])
                .title_bar(false)
                .show(ui, |ui| {
                    if ui.button("go").clicked() {
                        self.clicked += 1;
                    }
                });
        }

        fn report(&self) -> serde_json::Value {
            serde_json::json!({"updates": self.updates})
        }
    }

    struct BadReport;

    impl Game for BadReport {
        fn init(_: &mut Context) -> Self {
            Self
        }
        fn update(&mut self, _: &mut Context) {}
        fn draw(&mut self, _: &mut Context, _: f32) {}
        fn report(&self) -> serde_json::Value {
            serde_json::json!(7)
        }
    }

    fn run<G: Game>(gpu: &Gpu, tick_hz: u32, script: &str) -> Runner<G> {
        let script = Script::parse(script).unwrap();
        let mut runner = Runner::<G>::new(gpu, &config(tick_hz), &script).unwrap();
        runner.drive(None).unwrap();
        runner
    }

    #[test]
    fn raw_input_is_in_points_with_time_from_tick() {
        let raw = raw_input((200, 100), 2.0, 5, STEP, true);
        assert_eq!(
            raw.screen_rect,
            Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(100.0, 50.0)
            ))
        );
        assert_eq!(raw.time, Some(0.05));
        assert_eq!(raw.predicted_dt, 0.01);
        assert!(raw.focused);
        assert!(!raw_input((1, 1), 1.0, 0, STEP, false).focused);
        assert_eq!(
            raw.viewports[&raw.viewport_id].native_pixels_per_point,
            Some(2.0)
        );
    }

    #[test]
    fn pointer_coordinates_divide_by_scale_once() {
        let at = Some(Vec2::new(40.0, 20.0));
        assert_eq!(
            egui_event(&InputEvent::Cursor(at), None, 2.0),
            Some(egui::Event::PointerMoved(egui::pos2(20.0, 10.0)))
        );
        let click = InputEvent::Button {
            button: MouseButton::Left,
            state: Pressed,
        };
        assert_eq!(
            egui_event(&click, at, 2.0),
            Some(egui::Event::PointerButton {
                pos: egui::pos2(20.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            })
        );
        assert_eq!(egui_event(&click, None, 2.0), None);
    }

    #[test]
    fn every_input_event_translates() {
        let key = |code, state, repeat| InputEvent::Key {
            code,
            state,
            repeat,
        };
        assert_eq!(
            egui_event(&key(KeyCode::KeyA, Released, true), None, 1.0),
            Some(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: false,
                repeat: true,
                modifiers: egui::Modifiers::NONE,
            })
        );
        assert_eq!(
            egui_event(&key(KeyCode::Lang1, Pressed, false), None, 1.0),
            None
        );
        assert_eq!(
            egui_event(&InputEvent::Cursor(None), None, 1.0),
            Some(egui::Event::PointerGone)
        );
        assert_eq!(
            egui_event(&InputEvent::Motion(Vec2::new(4.0, 2.0)), None, 2.0),
            Some(egui::Event::MouseMoved(egui::vec2(2.0, 1.0)))
        );
        assert_eq!(
            egui_event(&InputEvent::Wheel(Vec2::new(0.0, 3.0)), None, 2.0),
            Some(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, 3.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            })
        );
        assert_eq!(
            egui_event(&InputEvent::Focus(false), None, 1.0),
            Some(egui::Event::WindowFocused(false))
        );
    }

    #[test]
    fn buttons_and_keys_map_to_egui() {
        let map = [
            (MouseButton::Right, Some(egui::PointerButton::Secondary)),
            (MouseButton::Middle, Some(egui::PointerButton::Middle)),
            (MouseButton::Back, Some(egui::PointerButton::Extra1)),
            (MouseButton::Forward, Some(egui::PointerButton::Extra2)),
            (MouseButton::Other(9), None),
        ];
        for (button, expected) in map {
            assert_eq!(egui_button(button), expected);
        }
        assert_eq!(egui_key(KeyCode::Digit1), Some(egui::Key::Num1));
        assert_eq!(egui_key(KeyCode::ArrowUp), Some(egui::Key::ArrowUp));
        assert_eq!(egui_key(KeyCode::Escape), Some(egui::Key::Escape));
    }

    #[test]
    fn run_error_texts_and_adapter_conversion() {
        let size = RunError::Size {
            size: (9, 8),
            max: 4,
        };
        assert_eq!(
            size.to_string(),
            "размер сценария 9x8 больше лимита текстуры 4"
        );
        assert_eq!(
            RunError::Report { tick: 3 }.to_string(),
            "тик 3: Game::report вернул не объект"
        );
        let e = RunError::from(wgpu::RequestAdapterError::NotFound {
            active_backends: wgpu::Backends::empty(),
            requested_backends: wgpu::Backends::empty(),
            supported_backends: wgpu::Backends::empty(),
            no_fallback_backends: wgpu::Backends::empty(),
            no_adapter_backends: wgpu::Backends::empty(),
            incompatible_surface_backends: wgpu::Backends::empty(),
        });
        assert!(matches!(e, RunError::NoAdapter(_)), "{e}");
        assert!(e.to_string().starts_with("нет адаптера GPU"));
    }

    #[test]
    fn tick_zero_has_no_update_and_snapshot_sees_its_update() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let script = Script::parse("ticks = 5\nsnapshots = [0, 2]").unwrap();
        let mut runner = Runner::<Toy>::new(&gpu, &config(60), &script).unwrap();
        assert!(runner.game.init_focused);
        assert_eq!(runner.step(), Some(0));
        assert_eq!((runner.game.updates, runner.game.draws), (0, 1));
        runner.snapshot(None).unwrap();
        for tick in 1..=5 {
            assert_eq!(runner.step(), Some(tick));
            if tick == 2 {
                runner.snapshot(None).unwrap();
            }
        }
        assert_eq!((runner.game.updates, runner.game.draws), (5, 6));
        assert_eq!(runner.step(), None);
        let report = runner.finish();
        let updates: Vec<_> = report
            .snapshots
            .iter()
            .map(|s| (s.tick, s.report["updates"].as_u64().unwrap()))
            .collect();
        assert_eq!(updates, [(0, 0), (2, 2)]);
        assert_eq!(report.ticks, 5);
        assert_eq!(report.snapshots[0].texts, ["hud"]);
        assert_eq!(report.snapshots[0].png, None);
    }

    #[test]
    fn advance_snapshots_listed_ticks_and_ends_with_none() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let script = Script::parse("ticks = 3\nsnapshots = [1]").unwrap();
        let mut runner = Runner::<Toy>::new(&gpu, &config(60), &script).unwrap();
        assert_eq!(runner.tick(), 0);
        let mut seen = Vec::new();
        for _ in 0..4 {
            let tick = runner.advance(None).unwrap().unwrap();
            seen.push((tick, runner.tick()));
        }
        assert_eq!(runner.advance(None).unwrap(), None);
        assert_eq!(seen, [(0, 0), (1, 1), (2, 2), (3, 3)]);
        assert_eq!(runner.canvas().size(), (64, 64));
        let ticks: Vec<_> = runner.finish().snapshots.iter().map(|s| s.tick).collect();
        assert_eq!(ticks, [1, 3]);
    }

    #[test]
    fn closed_error_names_tick() {
        assert_eq!(
            RunError::Closed { tick: 4 }.to_string(),
            "окно закрыто на тике 4, сценарий не кончился"
        );
    }

    #[test]
    fn debug_shows_message() {
        for e in [RunError::Closed { tick: 4 }, RunError::Stalled { tick: 4 }] {
            assert_eq!(format!("{e:?}"), e.to_string());
        }
    }

    #[test]
    fn stall_starts_at_timeout() {
        assert!(!stalled(Duration::from_millis(9_999)));
        assert!(stalled(STALL_TIMEOUT));
        assert!(stalled(STALL_TIMEOUT * 2));
    }

    #[test]
    fn stalled_error_names_tick_and_timeout() {
        let text = RunError::Stalled { tick: 7 }.to_string();
        assert!(text.contains("нет прогресса 10 с на тике 7"), "{text}");
        assert!(!text.contains("композитор"), "{text}");
        assert!(!text.contains("NO_VSYNC"), "{text}");
    }

    #[test]
    fn one_hz_still_updates_once_per_tick() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let runner = run::<Toy>(&gpu, 1, "ticks = 4");
        assert_eq!(runner.game.updates, 4);
    }

    #[test]
    fn panel_click_stays_out_of_input_but_release_reaches_it() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let runner = run::<Toy>(
            &gpu,
            60,
            r#"
            ticks = 12
            [[event]]
            tick = 3
            cursor = [30.0, 25.0]
            [[event]]
            tick = 4
            button_press = "Left"
            [[event]]
            tick = 5
            button_release = "Left"
            [[event]]
            tick = 7
            cursor = [60.0, 60.0]
            [[event]]
            tick = 8
            button_press = "Left"
            [[event]]
            tick = 9
            cursor = [30.0, 25.0]
            [[event]]
            tick = 10
            button_release = "Left"
            "#,
        );
        let g = &runner.game;
        assert_eq!(g.clicked, 1);
        assert_eq!(g.presses, 1);
        let downs: Vec<_> = g
            .downs
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| d.then_some(i + 1))
            .collect();
        assert_eq!(downs, [8, 9]);
    }

    #[test]
    fn cursor_queries_use_previous_frame() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let runner = run::<Toy>(
            &gpu,
            60,
            "ticks = 4\n[[event]]\ntick = 1\ncursor = [32.0, 32.0]",
        );
        let expected: Vec<_> = (0..4_i16).map(|k| Some(f32::from(k))).collect();
        for got in [&runner.game.worlds, &runner.game.rays] {
            assert!(
                got.iter()
                    .zip(&expected)
                    .all(|(g, e)| (g.unwrap() - e.unwrap()).abs() < 1e-3),
                "{got:?}"
            );
        }
    }

    #[test]
    fn motion_works_after_grab_and_is_dropped_in_grab_tick() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let script = Script::parse(
            "ticks = 3\n[[event]]\ntick = 1\nmotion = [1.0, 0.0]\n[[event]]\ntick = 2\nmotion = [0.0, 5.0]",
        )
        .unwrap();
        let mut runner = Runner::<Toy>::new(&gpu, &config(60), &script).unwrap();
        runner.game.grab = true;
        runner.drive(None).unwrap();
        assert_eq!(
            runner.game.deltas,
            [Vec2::ZERO, Vec2::new(0.0, 5.0), Vec2::ZERO]
        );
        assert!(runner.ctx.focused);
    }

    #[test]
    fn non_object_report_names_tick() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let script = Script::parse("ticks = 2\nsnapshots = [1]").unwrap();
        let mut runner = Runner::<BadReport>::new(&gpu, &config(60), &script).unwrap();
        let err = runner.drive(None).unwrap_err();
        assert!(matches!(err, RunError::Report { tick: 1 }), "{err}");
    }

    #[test]
    fn size_over_texture_limit_is_error() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let max = gpu.device.limits().max_texture_dimension_2d;
        let script = Script::parse(&format!("ticks = 1\nsize = [10, {}]", max + 1)).unwrap();
        let err = Runner::<Toy>::new(&gpu, &config(60), &script)
            .err()
            .unwrap();
        assert!(
            matches!(err, RunError::Size { max: m, .. } if m == max),
            "{err}"
        );
        let ok = Script::parse(&format!("ticks = 1\nsize = [{max}, 1]")).unwrap();
        assert!(Runner::<Toy>::new(&gpu, &config(60), &ok).is_ok());
    }

    #[test]
    fn png_files_are_written_and_overwritten() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("stapel-runner-{}", std::process::id()));
        let script = Script::parse("ticks = 2\nsnapshots = [0]").unwrap();
        for _ in 0..2 {
            let mut runner = Runner::<Toy>::new(&gpu, &config(60), &script).unwrap();
            runner.drive(Some(&dir)).unwrap();
            let report = runner.finish();
            assert_eq!(report.snapshots[1].png.as_deref(), Some("tick-000002.png"));
        }
        let img = image::open(dir.join("tick-000000.png")).unwrap();
        assert_eq!((img.width(), img.height()), (64, 64));
        fs::remove_dir_all(dir).unwrap();
    }
}
