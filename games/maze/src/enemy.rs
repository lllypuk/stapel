use engine::{IVec2, Vec2};

use crate::map::{Dir, Map};
use crate::player;

/// Скорость патруля, м/с.
pub(crate) const SPEED: f32 = 1.5;
/// Радиус круга столкновений, м.
pub(crate) const RADIUS: f32 = 0.25;

/// Орк на плане: ходит по оси от упора до упора.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Orc {
    /// Позиция прошлого тика, для интерполяции.
    pub(crate) prev: Vec2,
    pub(crate) pos: Vec2,
    /// Единичный, вдоль оси X или Z.
    pub(crate) dir: Vec2,
    /// Путь с начала уровня, м; по нему фаза `walk`.
    pub(crate) walked: f32,
}

impl Orc {
    /// В центре клетки `cell`, идёт к первой соседней клетке пола по порядку N, E, S, W.
    pub(crate) fn new(map: &Map, cell: IVec2) -> Self {
        let dir = Dir::ALL
            .into_iter()
            .find(|d| map.floor(cell + d.offset()))
            .unwrap_or(Dir::N);
        let pos = cell.as_vec2() + 0.5;
        Self {
            prev: pos,
            pos,
            dir: dir.offset().as_vec2(),
            walked: 0.0,
        }
    }

    /// Тик: шаг вперёд; упёрся в твёрдую клетку — встаёт вплотную и разворачивается.
    pub(crate) fn update(&mut self, dt: f32, solid: impl Fn(IVec2) -> bool) {
        self.prev = self.pos;
        let next = self.pos + self.dir * SPEED * dt;
        let front = (next + self.dir * RADIUS).floor().as_ivec2();
        if solid(front) {
            let edge = front.as_vec2() + 0.5 - self.dir * 0.5;
            self.pos += self.dir * (self.dir.dot(edge - self.pos) - RADIUS);
            self.dir = -self.dir;
        } else {
            self.pos = next;
        }
        self.walked += self.prev.distance(self.pos);
    }

    /// Позиция между тиками.
    pub(crate) fn at(&self, alpha: f32) -> Vec2 {
        self.prev.lerp(self.pos, alpha)
    }

    /// Путь между тиками.
    pub(crate) fn walked_at(&self, alpha: f32) -> f32 {
        self.walked - (1.0 - alpha) * self.prev.distance(self.pos)
    }

    /// Поворот вокруг +Y, переводящий +Z (перед модели glTF) в направление хода.
    pub(crate) fn yaw(&self) -> f32 {
        self.dir.x.atan2(self.dir.y)
    }

    pub(crate) fn touches(&self, player: Vec2) -> bool {
        self.pos.distance(player) < RADIUS + player::RADIUS
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use engine::{Quat, Vec3, ivec2, vec2, vec3};

    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn orc(text: &str) -> (Orc, Map) {
        let map = Map::parse(text).unwrap();
        (Orc::new(&map, map.orcs[0]), map)
    }

    /// Тики до разворота; дольше 10 с — паника.
    fn until_turn(o: &mut Orc, solid: impl Fn(IVec2) -> bool) {
        let dir = o.dir;
        for _ in 0..600 {
            o.update(DT, &solid);
            if o.dir != dir {
                return;
            }
        }
        panic!("не развернулся");
    }

    #[test]
    fn heads_to_first_open_neighbour() {
        for (text, dir) in [
            ("#.#\n#E.\n#P#\n", vec2(0.0, -1.0)),
            ("###\n#E.\n#P#\n", vec2(1.0, 0.0)),
            ("###\nPE#\n#.#\n", vec2(0.0, 1.0)),
            ("###\nPE#\n###\n", vec2(-1.0, 0.0)),
            ("P##\n#E#\n###\n", vec2(0.0, -1.0)),
        ] {
            let (o, _) = orc(text);
            assert_eq!(o.dir, dir, "{text}");
            assert_eq!((o.prev, o.pos), (vec2(1.5, 1.5), vec2(1.5, 1.5)));
        }
    }

    #[test]
    fn patrols_axis_turning_at_walls() {
        let (mut o, map) = orc("#P..E..#\n");
        let solid = |c| !map.floor(c);
        assert_eq!(o.dir, vec2(1.0, 0.0));
        until_turn(&mut o, solid);
        assert!((o.pos.x - (7.0 - RADIUS)).abs() < 1e-5, "{:?}", o.pos);
        assert_eq!(o.dir, vec2(-1.0, 0.0));
        until_turn(&mut o, solid);
        assert!((o.pos.x - (1.0 + RADIUS)).abs() < 1e-5, "{:?}", o.pos);
        assert_eq!(o.pos.y, 0.5);
        assert_eq!(o.dir, vec2(1.0, 0.0));
        o.update(DT, solid);
        assert!((o.pos.x - (1.0 + RADIUS + SPEED * DT)).abs() < 1e-5);
    }

    #[test]
    fn patrols_along_z_and_turns_at_gate() {
        let (mut o, map) = orc("#P#\n#.#\n#E#\n#D#\n#.#\n");
        let gate = ivec2(1, 3);
        let solid = |c| !map.floor(c) || c == gate;
        assert_eq!(o.dir, vec2(0.0, -1.0));
        until_turn(&mut o, solid);
        assert!((o.pos.y - RADIUS).abs() < 1e-5, "{:?}", o.pos);
        until_turn(&mut o, solid);
        assert!((o.pos.y - (3.0 - RADIUS)).abs() < 1e-5, "{:?}", o.pos);
        assert_eq!(o.pos.x, 1.5);
    }

    #[test]
    fn walked_grows_with_path_through_turns() {
        let (mut o, map) = orc("#P.E#\n");
        let solid = |c| !map.floor(c);
        for _ in 0..200 {
            o.update(DT, solid);
        }
        // Два разворота; у упора шаг короче полного.
        let full = 200.0 * SPEED * DT;
        assert!(
            o.walked <= full + 1e-4 && o.walked > full - 2.0 * SPEED * DT,
            "{}",
            o.walked
        );
    }

    #[test]
    fn draw_state_interpolates_between_ticks() {
        let o = Orc {
            prev: vec2(1.0, 2.0),
            pos: vec2(1.0, 2.5),
            dir: vec2(0.0, 1.0),
            walked: 3.0,
        };
        assert_eq!(o.at(0.25), vec2(1.0, 2.125));
        assert_eq!(o.walked_at(0.25), 3.0 - 0.75 * 0.5);
        assert_eq!(o.walked_at(1.0), 3.0);
    }

    #[test]
    fn yaw_turns_model_front_to_heading() {
        for (dir, yaw) in [
            (vec2(0.0, 1.0), 0.0),
            (vec2(1.0, 0.0), FRAC_PI_2),
            (vec2(0.0, -1.0), PI),
            (vec2(-1.0, 0.0), -FRAC_PI_2),
        ] {
            let o = Orc {
                dir,
                ..orc("PE\n").0
            };
            assert!((o.yaw() - yaw).abs() < 1e-6, "{dir:?}");
            let front = Quat::from_rotation_y(o.yaw()) * Vec3::Z;
            assert!(front.abs_diff_eq(vec3(dir.x, 0.0, dir.y), 1e-6));
        }
    }

    #[test]
    fn touches_within_both_radii() {
        let (o, _) = orc("PE\n");
        let reach = RADIUS + player::RADIUS;
        assert!(o.touches(o.pos + vec2(reach - 1e-3, 0.0)));
        assert!(!o.touches(o.pos + vec2(0.0, reach + 1e-3)));
        assert!(
            !o.touches(o.pos + vec2(reach, 0.0)),
            "вплотную — ещё не касание"
        );
    }
}
