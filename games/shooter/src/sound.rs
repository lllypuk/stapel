use stapel::{Assets, Camera3D, Handle, Sound, Stereo, Vec2, vec3};

use crate::CELL;

const SOUNDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sounds/");
const VOLUME: f32 = 0.6;
/// Метров до тишины.
const RANGE: f32 = 30.0;

/// Звуковое событие тика.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sfx {
    Blaster,
    Launcher,
    /// Выстрел стрелка.
    Bolt,
    Hit,
    Death,
    Pickup,
    Door,
}

const ALL: [Sfx; 7] = [
    Sfx::Blaster,
    Sfx::Launcher,
    Sfx::Bolt,
    Sfx::Hit,
    Sfx::Death,
    Sfx::Pickup,
    Sfx::Door,
];

impl Sfx {
    fn file(self) -> &'static str {
        match self {
            Sfx::Blaster => "blaster",
            Sfx::Launcher => "launcher",
            Sfx::Bolt => "bolt",
            Sfx::Hit => "hit",
            Sfx::Death => "death",
            Sfx::Pickup => "pickup",
            Sfx::Door => "door",
        }
    }
}

/// Событие в точке плана, клетки.
pub(crate) type Event = (Sfx, Vec2);

/// Звуки событий в порядке `ALL`.
pub(crate) struct Sounds([Handle<Sound>; 7]);

impl Sounds {
    pub(crate) fn load(assets: &mut Assets) -> Self {
        Self(ALL.map(|sfx| assets.load_sound(format!("{SOUNDS}{}.ogg", sfx.file()))))
    }

    pub(crate) fn get(&self, sfx: Sfx) -> Handle<Sound> {
        self.0[sfx as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SoundCmd {
    pub(crate) sfx: Sfx,
    pub(crate) gains: Stereo,
}

/// Команды событий тика для `listener`; неслышные отброшены.
pub(crate) fn commands(events: &[Event], listener: &Camera3D) -> Vec<SoundCmd> {
    events
        .iter()
        .filter_map(|&(sfx, p)| {
            // На высоте уха: расстояние и панорама — только по плану.
            let source = vec3(p.x * CELL, listener.pos.y, p.y * CELL);
            let gains = Stereo::at(listener, source, VOLUME, RANGE);
            (gains.left + gains.right > 0.0).then_some(SoundCmd { sfx, gains })
        })
        .collect()
}

/// Попадание или смерть по здоровью до и после тика.
pub(crate) fn wound(before: i32, after: i32, pos: Vec2) -> Option<Event> {
    match after {
        _ if after >= before => None,
        0 => Some((Sfx::Death, pos)),
        _ => Some((Sfx::Hit, pos)),
    }
}

#[cfg(test)]
mod tests {
    use stapel::vec2;

    use super::*;

    fn listener() -> Camera3D {
        Camera3D {
            pos: vec3(2.0 * CELL, 1.5, 3.0 * CELL),
            ..Camera3D::default()
        }
    }

    #[test]
    fn each_sfx_decodes_its_own_file() {
        let mut assets = Assets::default();
        let sounds = Sounds::load(&mut assets);
        for sfx in ALL {
            let file = assets.load_sound(format!("{SOUNDS}{}.ogg", sfx.file()));
            assert_eq!(sounds.get(sfx), file, "{sfx:?}");
            assert!(!assets.sound(file).is_empty(), "{sfx:?}");
        }
        let mut files = ALL.map(Sfx::file).to_vec();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), ALL.len());
    }

    #[test]
    fn source_is_placed_on_plan_at_ear_height() {
        let at = vec2(4.0, 1.0);
        let [cmd] = commands(&[(Sfx::Bolt, at)], &listener())[..] else {
            panic!("одна команда");
        };
        let source = vec3(at.x * CELL, 1.5, at.y * CELL);
        assert_eq!(cmd.sfx, Sfx::Bolt);
        assert_eq!(cmd.gains, Stereo::at(&listener(), source, VOLUME, RANGE));
    }

    #[test]
    fn source_on_the_right_is_louder_on_the_right() {
        // yaw 0 смотрит по −Z: справа — +X.
        let cmds = commands(&[(Sfx::Hit, vec2(5.0, 3.0))], &listener());
        assert!(cmds[0].gains.right > cmds[0].gains.left, "{cmds:?}");
    }

    #[test]
    fn own_sound_is_centred_at_full_volume() {
        let cmds = commands(&[(Sfx::Blaster, vec2(2.0, 3.0))], &listener());
        let g = cmds[0].gains;
        assert_eq!(g.left, g.right);
        assert!((g.left - VOLUME * 0.5f32.sqrt()).abs() < 1e-6, "{g:?}");
    }

    #[test]
    fn out_of_range_sends_nothing() {
        let edge = vec2(2.0 + RANGE / CELL, 3.0);
        let near = vec2(2.0 + RANGE / CELL - 0.5, 3.0);
        let cmds = commands(&[(Sfx::Hit, edge), (Sfx::Door, near)], &listener());
        assert_eq!(cmds.iter().map(|c| c.sfx).collect::<Vec<_>>(), [Sfx::Door]);
    }

    #[test]
    fn wound_is_hit_or_death_only_when_health_drops() {
        let p = vec2(1.0, 2.0);
        assert_eq!(wound(100, 100, p), None);
        assert_eq!(wound(50, 75, p), None);
        assert_eq!(wound(100, 75, p), Some((Sfx::Hit, p)));
        assert_eq!(wound(100, 1, p), Some((Sfx::Hit, p)));
        assert_eq!(wound(25, 0, p), Some((Sfx::Death, p)));
    }
}
