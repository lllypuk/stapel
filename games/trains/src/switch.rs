use engine::{Ray, vec2};

use crate::track::Track;
use crate::train::Train;

/// Дальность поиска стрелки впереди состава по пути, м.
pub(crate) const LOOKAHEAD: f32 = 24.0;
/// Радиус клика вокруг точки стрелки, м.
pub(crate) const PICK: f32 = 2.0;

/// Точка стрелки — стык внутри маршрута состава; концы состава ровно на стыке не в счёт.
pub(crate) fn under(track: &Track, train: &Train, switch: usize) -> bool {
    let port = track.switches[switch].port;
    let route = &train.route;
    (route.iter().take(route.len() - 1)).any(|&pass| track.exit(pass) == port)
}

/// Ближайшая стрелка за ведущим концом по последнему направлению хода, не дальше `LOOKAHEAD`.
pub(crate) fn ahead(track: &Track, train: &Train) -> Option<usize> {
    let (mut pass, mut dist) = if train.reverse {
        let (edge, forward) = train.route[0];
        ((edge, !forward), train.tail)
    } else {
        let last = train.route[train.route.len() - 1];
        (last, track.edges[last.0].len() - train.head)
    };
    loop {
        if dist > LOOKAHEAD {
            return None;
        }
        if let Some(i) = track.switch_at(track.exit(pass)) {
            return Some(i);
        }
        pass = track.next(pass)?;
        dist += track.edges[pass.0].len();
    }
}

/// Ближайшая стрелка в радиусе `PICK` от точки луча на земле y = 0; луч вдоль земли даёт NaN или ∞ и мимо.
pub(crate) fn pick(track: &Track, ray: Ray) -> Option<usize> {
    let t = -ray.origin.y / ray.dir.y;
    if t < 0.0 {
        return None;
    }
    let hit = ray.origin + ray.dir * t;
    let hit = vec2(hit.x, hit.z);
    (track.switches.iter().enumerate())
        .map(|(i, s)| (i, track.ports[s.port].pos.distance(hit)))
        .filter(|&(_, d)| d <= PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Перевод на другую ветку; `false` — стрелка под составом.
pub(crate) fn toggle(track: &mut Track, train: &Train, switch: usize) -> bool {
    if under(track, train, switch) {
        return false;
    }
    let s = &mut track.switches[switch];
    s.active = 1 - s.active;
    true
}

#[cfg(test)]
mod tests {
    use engine::{Assets, Vec2, Vec3, vec3};

    use super::*;
    use crate::DT;
    use crate::consist::{self, Car};
    use crate::piece::Piece;
    use crate::track::fixture::{edge, map};
    use crate::track::level01::{A, B, C, D};
    use crate::track::{LEVEL01, Pass};

    fn car(length: f32) -> Vec<Car> {
        let car = consist::load(&mut Assets::default(), "train/train-carriage-box");
        vec![Car { length, ..car }]
    }

    fn level01() -> Track {
        Track::parse(LEVEL01, 0.0).expect("level01")
    }

    /// Прямая петли низа z = 0, начинающаяся в x.
    fn bottom(t: &Track, x: i32) -> usize {
        edge(t, Piece::Straight, [x / 2, 0])
    }

    fn train(t: &Track, start: Pass, tail: f32, length: f32) -> Train {
        Train::new(t, start, tail, car(length)).expect("состав")
    }

    fn moved(mut train: Train, speed: f32, t: &Track) -> Train {
        train.speed = speed;
        train.advance(DT, t);
        train
    }

    #[test]
    fn key_finds_nearest_switch_ahead_by_last_direction() {
        let t = level01();
        let mut tr = train(&t, (bottom(&t, 20), true), 0.0, 2.0);
        assert_eq!(ahead(&t, &tr), Some(t.switch_by_pos(B)));
        tr = moved(tr, -1.0, &t);
        assert_eq!(ahead(&t, &tr), Some(t.switch_by_pos(A)));
        tr = moved(tr, 0.0, &t);
        assert_eq!(ahead(&t, &tr), Some(t.switch_by_pos(A)));
        tr = moved(tr, 1.0, &t);
        assert_eq!(ahead(&t, &tr), Some(t.switch_by_pos(B)));

        let back = train(&t, (bottom(&t, 4), false), 0.0, 2.0);
        assert_eq!(ahead(&t, &back), None, "тупика нет, петля за LOOKAHEAD");
        let depot = train(&t, t.depot[0], 0.5, 2.0);
        assert_eq!(ahead(&t, &depot), Some(t.switch_by_pos(D)));
        assert_eq!(ahead(&t, &moved(depot, -1.0, &t)), None, "тупик");
        let siding = edge(&t, Piece::Straight, [6, -1]);
        let tr = train(&t, (siding, true), 0.0, 1.0);
        assert_eq!(ahead(&t, &tr), Some(t.switch_by_pos(C)));
    }

    #[test]
    fn key_reaches_switch_exactly_at_lookahead() {
        let mut rails: Vec<_> = (0..8).map(|z| ("straight", [0, 2 * z], 0, false)).collect();
        rails.push(("curve", [0, 14], 0, false));
        let t = map(&rails);
        assert_eq!(ahead(&t, &train(&t, (0, true), 0.0, 4.0)), Some(0));
        assert_eq!(ahead(&t, &train(&t, (0, true), 0.0, 3.9)), None);
    }

    #[test]
    fn switch_under_train_is_locked() {
        let mut t = level01();
        let a = t.switch_by_pos(A);
        let cases = [
            (train(&t, (bottom(&t, 4), true), 2.0, 4.0), true),
            (train(&t, (bottom(&t, 8), false), 2.0, 4.0), true),
            (train(&t, (bottom(&t, 8), true), 0.0, 4.0), false),
            (train(&t, (bottom(&t, 4), true), 0.0, 4.0), false),
            (train(&t, (bottom(&t, 4), true), 4.0, 4.0), false),
        ];
        for (tr, locked) in cases {
            assert_eq!(under(&t, &tr, a), locked, "{tr:?}");
            let was = t.switches[a].active;
            assert_eq!(toggle(&mut t, &tr, a), !locked);
            assert_eq!(t.switches[a].active == was, locked);
        }
        let b = t.switch_by_pos(B);
        let tr = train(&t, (bottom(&t, 24), true), 2.0, 6.0);
        assert_eq!(tr.route.len(), 2);
        assert!(!under(&t, &tr, b), "голова ровно на стыке B");
        let mut tr = train(&t, (bottom(&t, 4), true), 0.0, 6.0);
        tr.speed = 1.0;
        tr.advance(4.0, &t);
        assert_eq!(tr.tail, 0.0, "хвост ровно на стыке A — уже за ним: {tr:?}");
        assert!(!under(&t, &tr, a));
        assert!(toggle(&mut t, &tr, b));
        assert_eq!(t.switches[b].active, 1);
    }

    fn ground(p: Vec2) -> Vec3 {
        vec3(p.x, 0.0, p.y)
    }

    #[test]
    fn click_by_ortho_ray_hits_switch_within_pick() {
        let t = level01();
        let ray = |p: Vec2| crate::camera(ground(p)).ray(Vec2::ZERO, 16.0 / 9.0);
        assert_eq!(pick(&t, ray(A)), Some(t.switch_by_pos(A)));
        assert_eq!(
            pick(&t, ray(C + vec2(0.0, PICK - 0.01))),
            Some(t.switch_by_pos(C))
        );
        assert_eq!(
            pick(&t, ray(D - vec2(PICK - 0.01, 0.0))),
            Some(t.switch_by_pos(D))
        );
        assert_eq!(pick(&t, ray(C + vec2(0.0, PICK + 0.01))), None);
        assert_eq!(pick(&t, ray(B + vec2(PICK + 0.01, 0.0))), None);
        let up = Ray {
            origin: ground(A) + Vec3::Y,
            dir: Vec3::Y,
        };
        assert_eq!(pick(&t, up), None);
        let flat = Ray {
            origin: ground(A) + Vec3::Y,
            dir: Vec3::X,
        };
        assert_eq!(pick(&t, flat), None);
        for y in [-1.0, 0.0] {
            let along = Ray {
                origin: ground(A) + Vec3::Y * y,
                dir: Vec3::X,
            };
            assert_eq!(pick(&t, along), None, "y = {y}: t = ∞ или NaN");
        }
        let below = Ray {
            origin: ground(B) - Vec3::Y,
            dir: Vec3::Y,
        };
        assert_eq!(pick(&t, below), Some(t.switch_by_pos(B)));
        let down = |p: Vec2, y: f32| Ray {
            origin: ground(p) + Vec3::Y * y,
            dir: Vec3::NEG_Y,
        };
        assert_eq!(pick(&t, down(A, 0.0)), Some(t.switch_by_pos(A)));
        assert_eq!(
            pick(&t, down(A + vec2(PICK, 0.0), 1.0)),
            Some(t.switch_by_pos(A))
        );
    }

    #[test]
    fn click_takes_nearest_of_close_switches() {
        let t = map(&[
            ("straight", [0, 0], 0, false),
            ("curve", [0, 0], 0, false),
            ("straight", [0, 2], 0, false),
            ("curve", [0, 2], 0, false),
        ]);
        assert_eq!(t.switches.len(), 2);
        let ray = |z: f32| Ray {
            origin: vec3(0.0, 1.0, z),
            dir: Vec3::NEG_Y,
        };
        let near = |z: f32| t.switch_by_pos(vec2(0.0, z));
        assert_eq!(pick(&t, ray(1.9)), Some(near(0.0)));
        assert_eq!(pick(&t, ray(2.1)), Some(near(4.0)));
    }
}
