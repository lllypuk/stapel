use glam::{IVec2, Vec2, ivec2};

use super::{Aabb, Tilemap};

/// Ось пиксельного шага.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Горизонталь.
    X,
    /// Вертикаль, вверх — плюс.
    Y,
}

/// Вход тела в сенсор на пиксельном шаге.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    /// Индекс в срезе сенсоров.
    pub sensor: usize,
    /// Ось шага, на котором началось пересечение.
    pub axis: Axis,
    /// Знак шага: `1` или `-1`.
    pub dir: i32,
}

/// По каким осям движение упёрлось в твёрдое.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hits {
    /// Упёрлось по X.
    pub x: bool,
    /// Упёрлось по Y.
    pub y: bool,
}

/// Тело, которое тайлы и `Solid` останавливают.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Actor {
    /// Положение в целых пикселях.
    pub aabb: Aabb,
    /// Накопленный дробный сдвиг, `|x|, |y| ≤ 0,5`.
    pub remainder: Vec2,
}

/// Сдвигает `actor` на `delta` с остатком; входы в `sensors` дописываются в `contacts`.
pub fn move_actor(
    map: &Tilemap,
    solids: &[Aabb],
    actor: &mut Actor,
    delta: Vec2,
    sensors: &[Aabb],
    contacts: &mut Vec<Contact>,
) -> Hits {
    actor.remainder += delta;
    let step = actor.remainder.round();
    actor.remainder -= step;
    let hits = move_exact(
        map,
        solids,
        &mut actor.aabb,
        step.as_ivec2(),
        sensors,
        contacts,
    );
    if hits.x {
        actor.remainder.x = 0.0;
    }
    if hits.y {
        actor.remainder.y = 0.0;
    }
    hits
}

/// Стоит ли `aabb` на тайле или `Solid`.
pub fn grounded(map: &Tilemap, solids: &[Aabb], aabb: &Aabb) -> bool {
    blocked(
        map,
        solids,
        &Aabb {
            min: aabb.min - IVec2::Y,
            ..*aabb
        },
    )
}

/// Целый сдвиг шагами по 1 px: сначала X, потом Y.
pub(super) fn move_exact(
    map: &Tilemap,
    solids: &[Aabb],
    aabb: &mut Aabb,
    step: IVec2,
    sensors: &[Aabb],
    contacts: &mut Vec<Contact>,
) -> Hits {
    let x = move_axis(map, solids, aabb, Axis::X, step.x, sensors, contacts);
    let y = move_axis(map, solids, aabb, Axis::Y, step.y, sensors, contacts);
    Hits { x, y }
}

fn move_axis(
    map: &Tilemap,
    solids: &[Aabb],
    aabb: &mut Aabb,
    axis: Axis,
    amount: i32,
    sensors: &[Aabb],
    contacts: &mut Vec<Contact>,
) -> bool {
    let dir = amount.signum();
    let unit = match axis {
        Axis::X => ivec2(dir, 0),
        Axis::Y => ivec2(0, dir),
    };
    for _ in 0..amount.abs() {
        let next = Aabb {
            min: aabb.min + unit,
            ..*aabb
        };
        if blocked(map, solids, &next) {
            return true;
        }
        for (sensor, s) in sensors.iter().enumerate() {
            if next.overlaps(s) && !aabb.overlaps(s) {
                contacts.push(Contact { sensor, axis, dir });
            }
        }
        *aabb = next;
    }
    false
}

fn blocked(map: &Tilemap, solids: &[Aabb], aabb: &Aabb) -> bool {
    map.overlaps_solid(aabb) || solids.iter().any(|s| s.overlaps(aabb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::vec2;

    /// Клетки 10×10, мир 100×60: пол, потолок, стена x 80..90 на y 20..40.
    fn level() -> Tilemap {
        let rows = [
            "##########",
            "..........",
            "........#.",
            "........#.",
            "..........",
            "##########",
        ];
        let solid = rows
            .iter()
            .rev()
            .flat_map(|r| r.bytes().map(|b| b == b'#'))
            .collect();
        Tilemap::new(10, 10, 6, solid)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Aabb {
        Aabb {
            min: ivec2(x, y),
            size: ivec2(w, h),
        }
    }

    fn body(x: i32, y: i32) -> Actor {
        Actor {
            aabb: rect(x, y, 8, 8),
            remainder: Vec2::ZERO,
        }
    }

    fn go(actor: &mut Actor, delta: Vec2, solids: &[Aabb], sensors: &[Aabb]) -> Vec<Contact> {
        let mut contacts = Vec::new();
        move_actor(&level(), solids, actor, delta, sensors, &mut contacts);
        contacts
    }

    fn contact(sensor: usize, axis: Axis, dir: i32) -> Contact {
        Contact { sensor, axis, dir }
    }

    #[test]
    fn remainder_accumulates_then_rounds_into_step() {
        let mut a = body(20, 20);
        go(&mut a, vec2(0.2, 0.0), &[], &[]);
        go(&mut a, vec2(0.2, 0.0), &[], &[]);
        assert_eq!(a.aabb.min, ivec2(20, 20));
        go(&mut a, vec2(0.2, 0.0), &[], &[]);
        assert_eq!(a.aabb.min, ivec2(21, 20));
        assert!((a.remainder.x + 0.4).abs() < 1e-5, "{}", a.remainder.x);
    }

    #[test]
    fn wall_stops_x_and_clears_only_x_remainder() {
        let mut a = body(60, 20);
        let mut contacts = Vec::new();
        let hits = move_actor(&level(), &[], &mut a, vec2(15.3, 0.3), &[], &mut contacts);
        assert_eq!(hits, Hits { x: true, y: false });
        assert_eq!(a.aabb.min, ivec2(72, 20));
        assert_eq!(a.remainder.x, 0.0);
        assert!((a.remainder.y - 0.3).abs() < 1e-6);
    }

    #[test]
    fn floor_stops_fall_and_clears_y_remainder() {
        let mut a = body(20, 15);
        let mut contacts = Vec::new();
        let hits = move_actor(&level(), &[], &mut a, vec2(0.0, -7.5), &[], &mut contacts);
        assert_eq!(hits, Hits { x: false, y: true });
        assert_eq!(a.aabb.min, ivec2(20, 10));
        assert_eq!(a.remainder, Vec2::ZERO);
    }

    #[test]
    fn ceiling_stops_rise_and_clears_y_remainder() {
        let mut a = body(20, 38);
        let mut contacts = Vec::new();
        let hits = move_actor(&level(), &[], &mut a, vec2(0.0, 6.4), &[], &mut contacts);
        assert_eq!(hits, Hits { x: false, y: true });
        assert_eq!(a.aabb.min, ivec2(20, 42));
        assert_eq!(a.remainder, Vec2::ZERO);
    }

    #[test]
    fn running_across_floor_tile_seams_is_not_stopped() {
        let mut a = body(5, 10);
        let mut contacts = Vec::new();
        let hits = move_actor(&level(), &[], &mut a, vec2(30.0, 0.0), &[], &mut contacts);
        assert_eq!(hits, Hits::default());
        assert_eq!(a.aabb.min, ivec2(35, 10));
    }

    #[test]
    fn fast_fall_does_not_tunnel_through_one_pixel_tile() {
        let solid = (0..20 * 20).map(|i| i / 20 == 5).collect();
        let map = Tilemap::new(1, 20, 20, solid);
        let mut a = Actor {
            aabb: rect(5, 12, 2, 2),
            remainder: Vec2::ZERO,
        };
        let mut contacts = Vec::new();
        for _ in 0..4 {
            move_actor(&map, &[], &mut a, vec2(0.0, -5.0), &[], &mut contacts);
            assert!(a.aabb.min.y >= 6, "{}", a.aabb.min.y);
        }
        assert_eq!(a.aabb.min.y, 6);
    }

    #[test]
    fn corner_resolves_x_before_y() {
        let mut a = body(20, 20);
        let mut contacts = Vec::new();
        let hits = move_actor(
            &level(),
            &[rect(28, 28, 8, 8)],
            &mut a,
            vec2(4.0, 4.0),
            &[],
            &mut contacts,
        );
        assert_eq!(hits, Hits { x: false, y: true });
        assert_eq!(a.aabb.min, ivec2(24, 20));
    }

    #[test]
    fn contact_records_axis_and_direction() {
        for (sensor, delta, expected) in [
            (rect(30, 20, 4, 4), vec2(5.0, 0.0), contact(0, Axis::X, 1)),
            (rect(14, 20, 4, 4), vec2(-5.0, 0.0), contact(0, Axis::X, -1)),
            (rect(20, 30, 4, 4), vec2(0.0, 5.0), contact(0, Axis::Y, 1)),
        ] {
            let mut a = body(20, 20);
            assert_eq!(go(&mut a, delta, &[], &[sensor]), [expected], "{delta}");
        }
    }

    #[test]
    fn contact_follows_pixel_path_not_swept_box() {
        let mut a = body(20, 20);
        assert_eq!(
            go(&mut a, vec2(5.0, 5.0), &[], &[rect(20, 29, 4, 4)]),
            Vec::<Contact>::new()
        );
        assert_eq!(a.aabb.min, ivec2(25, 25));
    }

    #[test]
    fn sensor_overlapped_before_step_is_not_repeated() {
        let mut a = body(20, 20);
        assert_eq!(
            go(&mut a, vec2(5.0, 0.0), &[], &[rect(22, 22, 20, 4)]),
            Vec::<Contact>::new()
        );
        let mut a = body(20, 20);
        let long = rect(30, 20, 20, 4);
        assert_eq!(
            go(&mut a, vec2(10.0, 0.0), &[], &[long]),
            [contact(0, Axis::X, 1)]
        );
    }

    #[test]
    fn two_sensors_in_one_tick_are_both_reported() {
        let mut a = body(20, 20);
        let sensors = [rect(40, 20, 2, 2), rect(30, 20, 2, 2)];
        assert_eq!(
            go(&mut a, vec2(15.0, 0.0), &[], &sensors),
            [contact(1, Axis::X, 1), contact(0, Axis::X, 1)]
        );
    }

    #[test]
    fn stomp_is_y_down_unlike_side_hit() {
        let enemy = rect(20, 12, 8, 4);
        let mut falling = body(20, 20);
        assert_eq!(
            go(&mut falling, vec2(0.0, -5.0), &[], &[enemy]),
            [contact(0, Axis::Y, -1)]
        );
        let mut running = body(10, 10);
        assert_eq!(
            go(&mut running, vec2(5.0, 0.0), &[], &[enemy]),
            [contact(0, Axis::X, 1)]
        );
    }

    #[test]
    fn blocked_step_reports_no_contact() {
        let mut a = body(72, 20);
        assert_eq!(
            go(&mut a, vec2(3.0, 0.0), &[], &[rect(80, 20, 4, 4)]),
            Vec::<Contact>::new()
        );
    }

    #[test]
    fn grounded_on_floor_and_platform_only() {
        let map = level();
        let platform = [rect(15, 22, 20, 8)];
        assert!(grounded(&map, &[], &rect(20, 10, 8, 8)));
        assert!(grounded(&map, &platform, &rect(20, 30, 8, 8)));
        assert!(!grounded(&map, &[], &rect(20, 30, 8, 8)));
        assert!(!grounded(&map, &platform, &rect(20, 31, 8, 8)));
        assert!(!grounded(&map, &[], &rect(72, 20, 8, 8)));
        assert!(!grounded(&map, &[], &rect(0, 20, 8, 8)));
    }
}
