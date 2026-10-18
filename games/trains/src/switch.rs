use engine::{Ray, vec2};

use crate::track::{Pass, Track};
use crate::train::Train;

/// Дальность поиска стрелки впереди состава по пути, м.
pub(crate) const LOOKAHEAD: f32 = 24.0;
/// Радиус клика вокруг точки стрелки, м.
pub(crate) const PICK: f32 = 2.0;

/// Точка стрелки — стык внутри маршрута состава; концы состава ровно на стыке не в счёт.
pub(crate) fn under(track: &Track, train: &Train, switch: usize) -> bool {
    let port = track.switches[switch].port;
    let route = &train.route;
    (route.iter().take(route.len() - 1)).any(|&pass| exit(track, pass) == port)
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
        let port = exit(track, pass);
        if let Some(i) = track.switches.iter().position(|s| s.port == port) {
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

fn exit(track: &Track, (edge, forward): Pass) -> usize {
    track.edges[edge].ends[usize::from(forward)].0
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use engine::{Assets, IVec2, Vec2, Vec3, vec3};

    use super::*;
    use crate::consist::Car;
    use crate::piece::Piece;
    use crate::track::LEVEL01;

    const A: Vec2 = vec2(8.0, 0.0);
    const B: Vec2 = vec2(32.0, 0.0);
    const C: Vec2 = vec2(16.0, -2.0);
    const D: Vec2 = vec2(12.0, 16.0);
    const DT: f32 = 1.0 / 60.0;

    fn car(length: f32) -> Vec<Car> {
        let mut assets = Assets::default();
        let h = assets.load_model(format!("{}train/train-carriage-box.glb", crate::ASSETS));
        let car = Car::new(h, assets.model(h)).expect("вагон");
        vec![Car { length, ..car }]
    }

    fn level01() -> Track {
        Track::parse(LEVEL01, 0.0).expect("level01")
    }

    /// Прямая петли низа z = 0, начинающаяся в x.
    fn bottom(t: &Track, x: i32) -> usize {
        (t.edges.iter())
            .position(|e| e.rail.piece == Piece::Straight && e.rail.cell == IVec2::new(x / 2, 0))
            .unwrap_or_else(|| panic!("{x}"))
    }

    fn at(t: &Track, pos: Vec2) -> usize {
        (t.switches.iter())
            .position(|s| t.ports[s.port].pos == pos)
            .unwrap_or_else(|| panic!("{pos}"))
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
        assert_eq!(ahead(&t, &tr), Some(at(&t, B)));
        tr = moved(tr, -1.0, &t);
        assert_eq!(ahead(&t, &tr), Some(at(&t, A)));
        tr = moved(tr, 0.0, &t);
        assert_eq!(ahead(&t, &tr), Some(at(&t, A)));
        tr = moved(tr, 1.0, &t);
        assert_eq!(ahead(&t, &tr), Some(at(&t, B)));

        let back = train(&t, (bottom(&t, 4), false), 0.0, 2.0);
        assert_eq!(ahead(&t, &back), None, "тупика нет, петля за LOOKAHEAD");
        let depot = train(&t, t.depot[0], 0.5, 2.0);
        assert_eq!(ahead(&t, &depot), Some(at(&t, D)));
        assert_eq!(ahead(&t, &moved(depot, -1.0, &t)), None, "тупик");
        let siding = (t.edges.iter())
            .position(|e| e.rail.cell == IVec2::new(6, -1))
            .expect("обгонный");
        let tr = train(&t, (siding, true), 0.0, 1.0);
        assert_eq!(ahead(&t, &tr), Some(at(&t, C)));
    }

    #[test]
    fn key_reaches_switch_exactly_at_lookahead() {
        let mut text = String::from("depot = { rail = 0 }\n");
        for z in [0, 2, 4, 6, 8, 10, 12] {
            writeln!(text, "[[rail]]\npiece = \"straight\"\ncell = [0, {z}]").expect("строка");
        }
        text.push_str("[[rail]]\npiece = \"straight\"\ncell = [0, 14]\n");
        text.push_str("[[rail]]\npiece = \"curve\"\ncell = [0, 14]\n");
        let t = Track::parse(&text, 0.0).expect("карта");
        assert_eq!(ahead(&t, &train(&t, (0, true), 0.0, 4.0)), Some(0));
        assert_eq!(ahead(&t, &train(&t, (0, true), 0.0, 3.9)), None);
    }

    #[test]
    fn switch_under_train_is_locked() {
        let mut t = level01();
        let a = at(&t, A);
        let cases = [
            (train(&t, (bottom(&t, 4), true), 2.0, 4.0), true),
            (train(&t, (bottom(&t, 8), false), 2.0, 4.0), true),
            (train(&t, (bottom(&t, 8), true), 0.0, 4.0), false),
            (train(&t, (bottom(&t, 4), true), 0.0, 4.0), false),
        ];
        for (tr, locked) in cases {
            assert_eq!(under(&t, &tr, a), locked, "{tr:?}");
            let was = t.switches[a].active;
            assert_eq!(toggle(&mut t, &tr, a), !locked);
            assert_eq!(t.switches[a].active == was, locked);
        }
        let b = at(&t, B);
        let tr = train(&t, (bottom(&t, 24), true), 2.0, 6.0);
        assert_eq!(tr.route.len(), 2);
        assert!(!under(&t, &tr, b), "голова ровно на стыке B");
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
        assert_eq!(pick(&t, ray(A)), Some(at(&t, A)));
        assert_eq!(pick(&t, ray(C + vec2(0.0, PICK - 0.01))), Some(at(&t, C)));
        assert_eq!(pick(&t, ray(D - vec2(PICK - 0.01, 0.0))), Some(at(&t, D)));
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
        let below = Ray {
            origin: ground(B) - Vec3::Y,
            dir: Vec3::Y,
        };
        assert_eq!(pick(&t, below), Some(at(&t, B)));
        let down = |p: Vec2, y: f32| Ray {
            origin: ground(p) + Vec3::Y * y,
            dir: Vec3::NEG_Y,
        };
        assert_eq!(pick(&t, down(A, 0.0)), Some(at(&t, A)));
        assert_eq!(pick(&t, down(A + vec2(PICK, 0.0), 1.0)), Some(at(&t, A)));
    }

    #[test]
    fn click_takes_nearest_of_close_switches() {
        let text = "depot = { rail = 0 }\n\
            [[rail]]\npiece = \"straight\"\ncell = [0, 0]\n\
            [[rail]]\npiece = \"curve\"\ncell = [0, 0]\n\
            [[rail]]\npiece = \"straight\"\ncell = [0, 2]\n\
            [[rail]]\npiece = \"curve\"\ncell = [0, 2]\n";
        let t = Track::parse(text, 0.0).expect("карта");
        assert_eq!(t.switches.len(), 2);
        let ray = |z: f32| Ray {
            origin: vec3(0.0, 1.0, z),
            dir: Vec3::NEG_Y,
        };
        let near = |z: f32| at(&t, vec2(0.0, z));
        assert_eq!(pick(&t, ray(1.9)), Some(near(0.0)));
        assert_eq!(pick(&t, ray(2.1)), Some(near(4.0)));
    }
}
