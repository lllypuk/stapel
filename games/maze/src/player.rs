use engine::{Camera3D, IVec2, Input, KeyCode, MAX_PITCH, Vec2, ivec2, vec3};

/// Высота глаза над полом, м.
pub(crate) const EYE: f32 = 0.7;
/// Радиус круга столкновений, м.
pub(crate) const RADIUS: f32 = 0.25;
/// Скорость ходьбы, м/с.
pub(crate) const SPEED: f32 = 3.0;
/// Радиан на единицу сырого сдвига мыши.
pub(crate) const SENSITIVITY: f32 = 0.0025;

/// Игрок на плане: `Vec2` — мировые X и Z, как у клетки столбец и строка.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Player {
    /// Позиция прошлого тика, для интерполяции.
    pub(crate) prev: Vec2,
    pub(crate) pos: Vec2,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
}

impl Player {
    pub(crate) fn new(pos: Vec2, yaw: f32) -> Self {
        Self {
            prev: pos,
            pos,
            yaw,
            pitch: 0.0,
        }
    }

    /// Тик: поворот на сдвиг мыши, затем шаг по WASD уже под новым углом.
    pub(crate) fn update(&mut self, input: &Input, dt: f32, solid: impl Fn(IVec2) -> bool) {
        (self.yaw, self.pitch) = turned(self.yaw, self.pitch, input.mouse_delta());
        self.prev = self.pos;
        let step = wish(input, self.yaw) * SPEED * dt;
        self.pos.x = slide(self.pos, step.x, 0, &solid);
        self.pos.y = slide(self.pos, step.y, 1, &solid);
    }

    /// Камера кадра: угол с ещё не потреблённым сдвигом `delta`, позиция между тиками.
    pub(crate) fn camera(&self, alpha: f32, delta: Vec2) -> Camera3D {
        let (yaw, pitch) = turned(self.yaw, self.pitch, delta);
        let p = self.prev.lerp(self.pos, alpha);
        Camera3D {
            pos: vec3(p.x, EYE, p.y),
            yaw,
            pitch,
            ..Camera3D::default()
        }
    }
}

/// Мышь вправо — поворот вправо, вверх (Y сдвига вниз) — взгляд вверх.
fn turned(yaw: f32, pitch: f32, delta: Vec2) -> (f32, f32) {
    let pitch = (pitch - delta.y * SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    (yaw - delta.x * SENSITIVITY, pitch)
}

/// Единичное направление ходьбы на плане или ноль.
fn wish(input: &Input, yaw: f32) -> Vec2 {
    let axis = |plus, minus| {
        f32::from(u8::from(input.is_down(plus))) - f32::from(u8::from(input.is_down(minus)))
    };
    let forward = Vec2::new(-yaw.sin(), -yaw.cos());
    let right = Vec2::new(yaw.cos(), -yaw.sin());
    (forward * axis(KeyCode::KeyW, KeyCode::KeyS) + right * axis(KeyCode::KeyD, KeyCode::KeyA))
        .normalize_or_zero()
}

/// Координата `axis` после сдвига на `d`, зажатая между твёрдыми клетками по обе стороны круга.
fn slide(pos: Vec2, d: f32, axis: usize, solid: impl Fn(IVec2) -> bool) -> f32 {
    let other = 1 - axis;
    let mut span = Vec2::splat(RADIUS);
    span[axis] += d.abs();
    let (lo, hi) = (
        (pos - span).floor().as_ivec2(),
        (pos + span).floor().as_ivec2(),
    );
    let (mut min, mut max) = (f32::NEG_INFINITY, f32::INFINITY);
    for y in lo.y..=hi.y {
        for x in lo.x..=hi.x {
            let cell = ivec2(x, y);
            let across = cell[other] as f32;
            let off = (across - pos[other])
                .max(pos[other] - across - 1.0)
                .max(0.0);
            if off >= RADIUS || !solid(cell) {
                continue;
            }
            let reach = (RADIUS * RADIUS - off * off).sqrt();
            let edge = cell[axis] as f32;
            if edge > pos[axis] {
                max = max.min(edge - reach);
            } else {
                min = min.max(edge + 1.0 + reach);
            }
        }
    }
    (pos[axis] + d).max(min).min(max)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI};

    use engine::{ElementState::Pressed, vec2};

    use super::*;
    use crate::map::{Dir, Map};

    const DT: f32 = 1.0 / 60.0;

    fn held(keys: &[KeyCode]) -> Input {
        let mut input = Input::default();
        for &k in keys {
            input.on_key(k, Pressed, false);
        }
        input
    }

    /// Игрок в центре клетки `P` карты; `ticks` тиков с зажатыми `keys`.
    fn walk(text: &str, yaw: f32, keys: &[KeyCode], ticks: u32) -> (Player, Map) {
        let map = Map::parse(text).unwrap();
        let mut p = Player::new(map.player.as_vec2() + 0.5, yaw);
        let input = held(keys);
        for _ in 0..ticks {
            p.update(&input, DT, |c| !map.floor(c));
        }
        (p, map)
    }

    /// Расстояние от центра игрока до ближайшей твёрдой клетки вокруг.
    fn clearance(p: Vec2, map: &Map) -> f32 {
        let c = p.floor().as_ivec2();
        (-1..=1)
            .flat_map(|y| (-1..=1).map(move |x| c + ivec2(x, y)))
            .filter(|&n| !map.floor(n))
            .map(|n| p.distance(p.clamp(n.as_vec2(), n.as_vec2() + 1.0)))
            .fold(f32::INFINITY, f32::min)
    }

    const OPEN: &str = ".....\n.....\n..P..\n.....\n.....\n";

    #[test]
    fn wasd_moves_relative_to_yaw() {
        let step = SPEED * DT;
        let start = vec2(2.5, 2.5);
        for (key, dir) in [
            (KeyCode::KeyW, vec2(1.0, 0.0)),
            (KeyCode::KeyS, vec2(-1.0, 0.0)),
            (KeyCode::KeyD, vec2(0.0, 1.0)),
            (KeyCode::KeyA, vec2(0.0, -1.0)),
        ] {
            let (p, _) = walk(OPEN, Dir::E.yaw(), &[key], 1);
            assert!(p.pos.abs_diff_eq(start + dir * step, 1e-6), "{key:?}");
            assert_eq!(p.prev, start);
        }
        let (p, _) = walk(OPEN, 0.0, &[KeyCode::KeyW, KeyCode::KeyD], 1);
        let diagonal = vec2(FRAC_1_SQRT_2, -FRAC_1_SQRT_2) * step;
        assert!(p.pos.abs_diff_eq(start + diagonal, 1e-6), "{:?}", p.pos);
        let (p, _) = walk(OPEN, 0.0, &[KeyCode::KeyW, KeyCode::KeyS], 1);
        assert_eq!(p.pos, start);
    }

    #[test]
    fn slides_along_wall() {
        // Взгляд на северо-восток в стену: по X идёт дальше, по Z прижат.
        let text = "#####\n.....\n.P...\n";
        let (p, map) = walk(text, -FRAC_1_SQRT_2.asin(), &[KeyCode::KeyW], 60);
        assert!((p.pos.y - (1.0 + RADIUS)).abs() < 1e-5, "{:?}", p.pos);
        assert!(p.pos.x > 3.0, "{:?}", p.pos);
        assert!((clearance(p.pos, &map) - RADIUS).abs() < 1e-5);
        // Назад от стены: тот же упор не держит.
        let (p, _) = walk(text, 0.0, &[KeyCode::KeyS], 10);
        assert!(p.pos.y > 2.5);
        // На юг и на восток.
        let (p, _) = walk("..P#\n####\n", PI, &[KeyCode::KeyW, KeyCode::KeyA], 60);
        assert!(
            p.pos.abs_diff_eq(vec2(3.0 - RADIUS, 1.0 - RADIUS), 1e-5),
            "{:?}",
            p.pos
        );
        let (p, _) = walk("#P.\n", Dir::W.yaw(), &[KeyCode::KeyW], 60);
        assert!((p.pos.x - (1.0 + RADIUS)).abs() < 1e-5, "{:?}", p.pos);
        // От стены вплотную — шаг целиком.
        let mut p = p;
        p.update(&held(&[KeyCode::KeyS]), DT, |c| c.x < 1);
        assert_eq!(p.pos.x, 1.0 + RADIUS + SPEED * DT);
    }

    #[test]
    fn off_centre_hit_stops_at_circle_contact() {
        // Угол стены (2, 1) — сбоку от линии хода, сверху и снизу.
        let input = held(&[KeyCode::KeyW]);
        for z in [0.9, 2.1] {
            let wall = |c: IVec2| c == ivec2(2, 1);
            let mut p = Player::new(vec2(0.5, z), Dir::E.yaw());
            for _ in 0..60 {
                p.update(&input, DT, wall);
            }
            let contact = 2.0 - (RADIUS * RADIUS - 0.01f32).sqrt();
            assert!((p.pos.x - contact).abs() < 1e-4, "{z}: {:?}", p.pos);
            assert!((p.pos.y - z).abs() < 1e-6);
        }
    }

    #[test]
    fn corner_is_not_passed_at_full_speed() {
        // Диагональная щель между стенами нулевой ширины, ход к ней по диагонали.
        let (p, map) = walk("#.\nP#\n", -PI / 4.0, &[KeyCode::KeyW], 120);
        assert_eq!(p.pos.floor().as_ivec2(), map.player);
        assert!(clearance(p.pos, &map) >= RADIUS - 1e-5);
        // Угол комнаты: оба упора сразу.
        let (p, map) = walk("###\n#P#\n###\n", -PI / 4.0, &[KeyCode::KeyW], 120);
        assert!(
            p.pos.abs_diff_eq(vec2(2.0 - RADIUS, 1.0 + RADIUS), 1e-5),
            "{:?}",
            p.pos
        );
        assert!(clearance(p.pos, &map) >= RADIUS - 1e-5);
    }

    #[test]
    fn tick_turns_by_mouse_and_clamps_pitch() {
        let mut input = Input::default();
        input.on_mouse_motion(vec2(40.0, -20.0));
        let mut p = Player::new(Vec2::ZERO, 1.0);
        p.update(&input, DT, |_| false);
        assert_eq!(p.yaw, 1.0 - 40.0 * SENSITIVITY);
        assert_eq!(p.pitch, 20.0 * SENSITIVITY);
        input.on_mouse_motion(vec2(0.0, -1e6));
        p.update(&input, DT, |_| false);
        assert_eq!(p.pitch, MAX_PITCH);
        input.end_tick();
        input.on_mouse_motion(vec2(0.0, 1e6));
        p.update(&input, DT, |_| false);
        assert_eq!(p.pitch, -MAX_PITCH);
    }

    #[test]
    fn walk_uses_angle_of_this_tick() {
        let mut input = held(&[KeyCode::KeyW]);
        input.on_mouse_motion(vec2(-FRAC_PI_2 / SENSITIVITY, 0.0));
        let mut p = Player::new(Vec2::ZERO, 0.0);
        p.update(&input, DT, |_| false);
        assert!(
            p.pos.abs_diff_eq(vec2(-SPEED * DT, 0.0), 1e-5),
            "{:?}",
            p.pos
        );
    }

    #[test]
    fn camera_interpolates_and_adds_unconsumed_delta() {
        let p = Player {
            prev: vec2(1.0, 2.0),
            pos: vec2(2.0, 4.0),
            yaw: 0.5,
            pitch: 0.1,
        };
        let cam = p.camera(0.25, vec2(10.0, 4.0));
        assert_eq!(cam.pos, vec3(1.25, EYE, 2.5));
        assert_eq!(cam.yaw, 0.5 - 10.0 * SENSITIVITY);
        assert_eq!(cam.pitch, 0.1 - 4.0 * SENSITIVITY);
        assert_eq!(cam.fov_y, Camera3D::default().fov_y);
    }
}
