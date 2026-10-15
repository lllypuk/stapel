use std::cmp::Ordering;

use glam::{IVec2, Vec2};

use super::actor::move_exact;
use super::{Aabb, Actor, Contact, Tilemap};

/// Кинематическое тело: проходит сквозь тайлы, толкает и везёт `Actor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solid {
    /// Положение в целых пикселях.
    pub aabb: Aabb,
    /// Накопленный дробный сдвиг, `|x|, |y| ≤ 0,5`.
    pub remainder: Vec2,
}

/// Сдвигает `solids[i]` на `delta`, толкая пересечённых и везя стоящих сверху `actors`.
/// Индекс в `contacts` и `squished` — Actor; `squished` — упёрся при толчке.
#[expect(
    clippy::too_many_arguments,
    reason = "физика — свободные функции над срезами тел игры"
)]
pub fn move_solid(
    map: &Tilemap,
    solids: &mut [Solid],
    i: usize,
    delta: Vec2,
    actors: &mut [Actor],
    sensors: &[Aabb],
    contacts: &mut Vec<(usize, Contact)>,
    squished: &mut Vec<usize>,
) {
    solids[i].remainder += delta;
    let step = solids[i].remainder.round();
    solids[i].remainder -= step;
    let step = step.as_ivec2();
    // Пассажиры — снимок до движения по обеим осям: выбор, см. docs/design.md «Физика».
    let riding: Vec<bool> = actors
        .iter()
        .map(|a| {
            Aabb {
                min: a.aabb.min - IVec2::Y,
                ..a.aabb
            }
            .overlaps(&solids[i].aabb)
        })
        .collect();
    let others: Vec<Aabb> = solids
        .iter()
        .enumerate()
        .filter(|&(j, _)| j != i)
        .map(|(_, s)| s.aabb)
        .collect();
    let mut found = Vec::new();
    for unit in [IVec2::X, IVec2::Y] {
        let amount = step.dot(unit);
        let forward = match amount.cmp(&0) {
            Ordering::Equal => continue,
            Ordering::Greater => true,
            Ordering::Less => false,
        };
        solids[i].aabb.min += unit * amount;
        let s = solids[i].aabb;
        for (j, actor) in actors.iter_mut().enumerate() {
            let pushed = actor.aabb.overlaps(&s);
            let shift = if pushed {
                let depth = if forward {
                    s.max() - actor.aabb.min
                } else {
                    s.min - actor.aabb.max()
                };
                depth * unit
            } else if riding[j] {
                amount * unit
            } else {
                continue;
            };
            let hits = move_exact(map, &others, &mut actor.aabb, shift, sensors, &mut found);
            contacts.extend(found.drain(..).map(|c| (j, c)));
            if pushed && (hits.x || hits.y) && !squished.contains(&j) {
                squished.push(j);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{Axis, grounded};
    use glam::{ivec2, vec2};

    /// Клетки 10×10, мир 100×60: пол, стена x 80..90 на y 10..40.
    fn level() -> Tilemap {
        let rows = [
            "..........",
            "..........",
            "........#.",
            "........#.",
            "........#.",
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

    fn platform(x: i32, y: i32, w: i32, h: i32) -> Solid {
        Solid {
            aabb: rect(x, y, w, h),
            remainder: Vec2::ZERO,
        }
    }

    struct Out {
        contacts: Vec<(usize, Contact)>,
        squished: Vec<usize>,
    }

    fn shift(solids: &mut [Solid], delta: Vec2, actors: &mut [Actor], sensors: &[Aabb]) -> Out {
        let mut out = Out {
            contacts: Vec::new(),
            squished: Vec::new(),
        };
        move_solid(
            &level(),
            solids,
            0,
            delta,
            actors,
            sensors,
            &mut out.contacts,
            &mut out.squished,
        );
        out
    }

    #[test]
    fn rider_travels_along_x_and_y_keeping_its_remainder() {
        let mut solids = [platform(20, 20, 20, 4)];
        let mut actors = [Actor {
            remainder: vec2(0.5, 0.5),
            ..body(25, 24)
        }];
        shift(&mut solids, vec2(3.0, 2.0), &mut actors, &[]);
        assert_eq!(solids[0].aabb.min, ivec2(23, 22));
        assert_eq!(actors[0].aabb.min, ivec2(28, 26));
        assert_eq!(actors[0].remainder, vec2(0.5, 0.5));
    }

    #[test]
    fn solid_keeps_its_own_remainder() {
        let mut solids = [platform(20, 20, 20, 4)];
        shift(&mut solids, vec2(0.6, 0.0), &mut [], &[]);
        assert_eq!(solids[0].aabb.min, ivec2(21, 20));
        assert!((solids[0].remainder.x + 0.4).abs() < 1e-6);
    }

    #[test]
    fn falling_platform_keeps_rider_on_it() {
        let mut solids = [platform(20, 20, 20, 4)];
        let mut actors = [body(25, 24)];
        shift(&mut solids, vec2(0.0, -5.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(25, 19));
        assert!(grounded(&level(), &[solids[0].aabb], &actors[0].aabb));
    }

    #[test]
    fn rising_platform_pushes_overlapped_and_carries_rider() {
        let mut solids = [platform(20, 20, 20, 4)];
        let mut actors = [body(22, 24), body(32, 26), body(25, 40)];
        shift(&mut solids, vec2(0.0, 4.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(22, 28));
        assert_eq!(actors[1].aabb.min, ivec2(32, 28));
        assert_eq!(actors[2].aabb.min, ivec2(25, 40));
    }

    #[test]
    fn side_push_moves_by_overlap_depth() {
        let mut solids = [platform(10, 10, 10, 10)];
        let mut actors = [
            Actor {
                remainder: vec2(0.5, 0.0),
                ..body(22, 10)
            },
            body(0, 10),
        ];
        let out = shift(&mut solids, vec2(5.0, 0.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(25, 10));
        assert_eq!(actors[0].remainder, vec2(0.5, 0.0));
        assert_eq!(actors[1].aabb.min, ivec2(0, 10));
        assert_eq!(out.squished, Vec::<usize>::new());

        let mut solids = [platform(30, 10, 10, 10)];
        let mut actors = [body(22, 10)];
        shift(&mut solids, vec2(-5.0, 0.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(17, 10));
    }

    #[test]
    fn push_into_wall_squishes_once() {
        let mut solids = [platform(60, 10, 8, 8)];
        let mut actors = [body(70, 10)];
        let out = shift(&mut solids, vec2(6.0, 0.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(72, 10));
        assert_eq!(out.squished, [0]);

        let mut solids = [platform(64, 12, 8, 8)];
        let mut actors = [body(72, 10)];
        let out = shift(&mut solids, vec2(2.0, -2.0), &mut actors, &[]);
        assert_eq!(out.squished, [0]);
    }

    #[test]
    fn carry_into_wall_stops_rider_without_squish() {
        let mut solids = [platform(60, 20, 16, 4)];
        let mut actors = [body(70, 24)];
        let out = shift(&mut solids, vec2(5.0, 0.0), &mut actors, &[]);
        assert_eq!(solids[0].aabb.min, ivec2(65, 20));
        assert_eq!(actors[0].aabb.min, ivec2(72, 24));
        assert_eq!(out.squished, Vec::<usize>::new());
    }

    #[test]
    fn actor_on_actor_is_not_a_passenger() {
        let mut solids = [platform(20, 20, 20, 4)];
        let mut actors = [body(25, 24), body(25, 32)];
        shift(&mut solids, vec2(3.0, 0.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(28, 24));
        assert_eq!(actors[1].aabb.min, ivec2(25, 32));
    }

    #[test]
    fn solid_passes_through_tiles() {
        let mut solids = [platform(60, 12, 8, 8)];
        shift(&mut solids, vec2(25.0, -7.0), &mut [], &[]);
        assert_eq!(solids[0].aabb.min, ivec2(85, 5));
    }

    #[test]
    fn other_solid_blocks_push_but_mover_does_not() {
        let mut solids = [platform(10, 10, 10, 10), platform(30, 10, 4, 10)];
        let mut actors = [body(20, 10)];
        let out = shift(&mut solids, vec2(5.0, 0.0), &mut actors, &[]);
        assert_eq!(actors[0].aabb.min, ivec2(22, 10));
        assert_eq!(out.squished, [0]);
    }

    #[test]
    fn contacts_and_squish_belong_to_their_actors() {
        let mut solids = [platform(60, 10, 8, 20)];
        let mut actors = [body(60, 30), body(70, 10)];
        let sensors = [rect(78, 10, 2, 2), rect(70, 30, 2, 2)];
        let out = shift(&mut solids, vec2(6.0, 0.0), &mut actors, &sensors);
        let contact = |sensor| Contact {
            sensor,
            axis: Axis::X,
            dir: 1,
        };
        assert_eq!(out.contacts, [(0, contact(1)), (1, contact(0))]);
        assert_eq!(out.squished, [1]);
    }
}
