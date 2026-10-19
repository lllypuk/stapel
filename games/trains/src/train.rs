use std::collections::VecDeque;

use engine::Vec2;

use crate::consist::{self, Car, GAP};
use crate::track::{MARGIN, Pass, Track};

/// Предел скорости в обе стороны, м/с.
pub(crate) const MAX_SPEED: f32 = 6.0;
/// Разгон одной машины, м/с²; с вагонами делится на их число.
const ACCEL: f32 = 3.0;
/// Торможение педалью против хода, м/с².
const BRAKE: f32 = 4.0;
/// Замедление накатом без педали, м/с².
const ROLLING: f32 = 0.3;

/// Машина на пути в плоскости карты.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    /// Передняя и задняя тележки на оси пути и касательные к голове.
    pub(crate) bogies: [(Vec2, Vec2); 2],
    /// Начало модели.
    pub(crate) pos: Vec2,
    /// +Z модели — хорда от задней тележки к передней.
    pub(crate) dir: Vec2,
}

/// Сцепка — от заднего края машины до переднего края следующей.
pub(crate) type Coupler = [Vec2; 2];

/// Состав на пути.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Train {
    /// Рёбра под составом от хвоста к голове, проезды — по ходу к голове.
    pub(crate) route: VecDeque<Pass>,
    /// Голова — от входа в последний проезд.
    pub(crate) head: f32,
    /// Хвост — от входа в первый проезд.
    pub(crate) tail: f32,
    /// Плюс — к голове.
    pub(crate) speed: f32,
    /// Последний ход — задний; на старте — вперёд.
    pub(crate) reverse: bool,
    pub(crate) cars: Vec<Car>,
}

impl Train {
    /// Хвост на `tail` от входа в `start`, голова — по активным веткам; `None` — путь кончился раньше.
    pub(crate) fn new(track: &Track, start: Pass, tail: f32, cars: Vec<Car>) -> Option<Self> {
        let mut route = VecDeque::from([start]);
        let mut head = tail + consist::len(&cars);
        loop {
            let last = route[route.len() - 1];
            let len = track.edges[last.0].len();
            if head <= len {
                break;
            }
            head -= len;
            route.push_back(track.next(last)?);
        }
        let mut train = Self {
            route,
            head,
            tail,
            speed: 0.0,
            reverse: false,
            cars,
        };
        train.trim(track);
        Some(train)
    }

    /// Педаль −1..1 по ходу разгоняет, против хода тормозит до остановки.
    pub(crate) fn drive(&mut self, pedal: f32, dt: f32) {
        let v = self.speed;
        self.speed = if pedal * v < 0.0 {
            toward_zero(v, BRAKE * pedal.abs() * dt)
        } else if pedal == 0.0 {
            toward_zero(v, ROLLING * dt)
        } else {
            let accel = ACCEL / self.cars.len().max(1) as f32;
            (v + pedal * accel * dt).clamp(-MAX_SPEED, MAX_SPEED)
        };
    }

    /// Сдвиг на `speed·dt`; перед тупиком или стрелкой на другой ветке — остановка за `MARGIN`.
    pub(crate) fn advance(&mut self, dt: f32, track: &Track) {
        let dist = self.speed * dt;
        let back = dist < 0.0;
        if dist != 0.0 {
            self.reverse = back;
        }
        if back {
            self.flip(track);
        }
        if self.lead(dist.abs(), track) {
            self.speed = 0.0;
        }
        if back {
            self.flip(track);
        }
    }

    /// Голова вперёд на `dist`, хвост — следом по `route`; `true` — голова упёрлась.
    fn lead(&mut self, mut dist: f32, track: &Track) -> bool {
        let mut moved = 0.0;
        let blocked = loop {
            let last = self.route[self.route.len() - 1];
            let len = track.edges[last.0].len();
            let next = track.next(last);
            let end = if next.is_some() { len } else { len - MARGIN };
            let step = dist.min((end - self.head).max(0.0));
            self.head += step;
            moved += step;
            dist -= step;
            if dist <= 0.0 {
                break false;
            }
            let Some(pass) = next else { break true };
            self.route.push_back(pass);
            self.head = 0.0;
        };
        self.tail += moved;
        self.trim(track);
        blocked
    }

    /// Снимает проезды позади хвоста; хвост ровно на выходе проезда — уже на следующем.
    fn trim(&mut self, track: &Track) {
        loop {
            let len = track.edges[self.route[0].0].len();
            if self.tail < len {
                break;
            }
            self.tail -= len;
            self.route.pop_front();
        }
    }

    /// Машины от головы и сцепки между ними.
    pub(crate) fn cars_at(&self, track: &Track) -> (Vec<Placed>, Vec<Coupler>) {
        let mut back = 0.0;
        let placed: Vec<_> = self
            .cars
            .iter()
            .map(|car| {
                let bogies = car.bogies.map(|b| self.at(track, back + b));
                back += car.length + GAP;
                let dir = (bogies[0].0 - bogies[1].0).normalize();
                Placed {
                    bogies,
                    pos: bogies[0].0 - dir * (car.front - car.bogies[0]),
                    dir,
                }
            })
            .collect();
        let couplers = placed
            .windows(2)
            .zip(self.cars.windows(2))
            .map(|(p, c)| {
                [
                    p[0].pos + p[0].dir * (c[0].front - c[0].length),
                    p[1].pos + p[1].dir * c[1].front,
                ]
            })
            .collect();
        (placed, couplers)
    }

    /// Точка оси и касательная к голове на `back` позади головы.
    fn at(&self, track: &Track, back: f32) -> (Vec2, Vec2) {
        let mut s = self.head - back;
        for (i, &(edge, forward)) in self.route.iter().enumerate().rev() {
            if s >= 0.0 || i == 0 {
                let edge = &track.edges[edge];
                if forward {
                    return edge.rail.point_at(s);
                }
                let (p, d) = edge.rail.point_at(edge.len() - s);
                return (p, -d);
            }
            s += track.edges[self.route[i - 1].0].len();
        }
        unreachable!("маршрут не пуст")
    }

    /// Хвост и голова меняются местами: задний ход — тот же ход головой.
    fn flip(&mut self, track: &Track) {
        let len = |p: Pass| track.edges[p.0].len();
        let (first, last) = (self.route[0], self.route[self.route.len() - 1]);
        (self.head, self.tail) = (len(first) - self.tail, len(last) - self.head);
        self.route.make_contiguous().reverse();
        for pass in &mut self.route {
            pass.1 = !pass.1;
        }
    }
}

fn toward_zero(v: f32, by: f32) -> f32 {
    v.signum() * (v.abs() - by).max(0.0)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use engine::{Assets, vec2};

    use super::*;
    use crate::piece::Piece;
    use crate::track::LEVEL01;
    use crate::track::level01::{A, B, C, set};

    const DT: f32 = 1.0 / 60.0;

    fn track(rails: &[(&str, [i32; 2], i64)]) -> Track {
        let rails: Vec<_> = rails.iter().map(|&(p, c, r)| (p, c, r, false)).collect();
        mirrored(&rails)
    }

    fn mirrored(rails: &[(&str, [i32; 2], i64, bool)]) -> Track {
        let mut text = String::from("depot = { rail = 0 }\n");
        for (piece, [x, z], rot, mirror) in rails {
            writeln!(
                text,
                "[[rail]]\npiece = \"{piece}\"\ncell = [{x}, {z}]\nrot = {rot}\nmirror = {mirror}"
            )
            .expect("строка");
        }
        Track::parse(&text, 0.0).expect("карта")
    }

    /// Шесть прямых по +Z от тупика (0, 0) до тупика (0, 24).
    fn line() -> Track {
        track(&[0, 2, 4, 6, 8, 10].map(|z| ("straight", [0, z], 0)))
    }

    /// Стык (0, −8)…(0, 0), стрелка (0, 0): ветка 2 — прямая до (0, 8) с ребром 4, ветка 3 — S до (−2, 8) с ребром 5.
    fn fork() -> Track {
        track(&[
            ("straight", [0, -4], 0),
            ("straight", [0, 0], 180),
            ("straight", [0, 0], 0),
            ("curve", [0, 0], 0),
            ("straight", [0, 2], 0),
            ("straight", [-1, 2], 0),
        ])
    }

    /// Машины длиной `length`.
    fn cars(n: usize, length: f32) -> Vec<Car> {
        let car = consist::load(&mut Assets::default(), "train/train-carriage-box");
        vec![Car { length, ..car }; n]
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn near2(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-4
    }

    /// Машины игры и прямоугольники их габаритов `[min, max]` по (x, z) модели.
    fn catalog() -> Vec<(Car, [Vec2; 2])> {
        let mut assets = Assets::default();
        (consist::all(&mut assets).into_iter())
            .map(|c| {
                let b = assets.model(c.model).bounds().expect("габариты");
                (c, [vec2(b.min.x, b.min.z), vec2(b.max.x, b.max.z)])
            })
            .collect()
    }

    /// Голова и хвост от начала прямой `line`.
    fn along(t: &Train) -> (f32, f32) {
        let at = |p: Pass, d: f32| {
            assert!(p.1, "{t:?}");
            p.0 as f32 * 4.0 + d
        };
        (
            at(t.route[t.route.len() - 1], t.head),
            at(t.route[0], t.tail),
        )
    }

    fn moved(mut t: Train, speed: f32, dt: f32, track: &Track) -> Train {
        t.speed = speed;
        t.advance(dt, track);
        t
    }

    #[test]
    fn new_lays_head_along_active_branches() {
        let l = line();
        let t = Train::new(&l, (0, true), 1.0, cars(3, 2.0)).expect("состав");
        assert_eq!(t.route, [(0, true), (1, true)]);
        assert!(near(t.head, 3.0 + 2.0 * GAP) && near(t.tail, 1.0), "{t:?}");
        assert_eq!(t.speed, 0.0);
        let t = Train::new(&l, (0, true), 2.0, cars(1, 2.0)).expect("состав");
        assert_eq!((t.route, t.head), ([(0, true)].into(), 4.0));
        assert_eq!(Train::new(&l, (5, true), 3.0, cars(2, 2.0)), None);
        let mut f = fork();
        f.switches[0].active = 1;
        let t = Train::new(&f, (0, true), 1.0, cars(4, 2.0)).expect("состав");
        assert_eq!(t.route, [(0, true), (1, false), (3, true)]);
        assert!(near(t.head, 1.0 + 3.0 * GAP), "{t:?}");
    }

    #[test]
    fn tick_crosses_several_joints_and_tail_shortens_route() {
        let l = line();
        let t = Train::new(&l, (0, true), 1.0, cars(1, 2.0)).expect("состав");
        let t = moved(t, 10.0, 1.0, &l);
        assert_eq!(t.route, [(2, true), (3, true)]);
        assert_eq!((t.head, t.tail), (1.0, 3.0));
        let t = moved(t, 3.0, 1.0, &l);
        assert_eq!((&t.route, t.head, t.tail), (&[(3, true)].into(), 4.0, 2.0));
        let t = moved(t, 2.0, 1.0, &l);
        assert_eq!(
            (&t.route, t.head, t.tail),
            (&[(4, true)].into(), 2.0, 0.0),
            "хвост ровно на стыке — уже за ним"
        );
    }

    #[test]
    fn reverse_leads_with_tail_and_keeps_route_order() {
        let l = line();
        let t = Train::new(&l, (3, true), 1.0, cars(1, 2.0)).expect("состав");
        let t = moved(t, -5.0, 2.0, &l);
        assert_eq!(t.route, [(0, true), (1, true)]);
        let (head, tail) = along(&t);
        assert!(near(head, 5.0) && near(tail, 3.0), "{t:?}");
        assert_eq!(t.speed, -5.0);
        let t = moved(t, -5.0, 1.0, &l);
        assert_eq!(t.route, [(0, true)]);
        let (head, tail) = along(&t);
        assert!(near(head, 2.0 + MARGIN) && near(tail, MARGIN), "{t:?}");
        assert_eq!(t.speed, 0.0);
    }

    #[test]
    fn switch_thrown_under_train_keeps_tail_on_its_branch() {
        let mut f = fork();
        f.switches[0].active = 1;
        let t = Train::new(&f, (0, true), 1.0, cars(2, 2.75)).expect("состав");
        let t = moved(t, 4.0, 1.0, &f);
        assert_eq!(t.route, [(1, false), (3, true)]);
        f.switches[0].active = 0;
        let back = moved(t.clone(), -2.0, 1.0, &f);
        assert_eq!(back.route, [(0, true), (1, false), (3, true)]);
        let t = moved(t, 4.0, 1.0, &f);
        assert_eq!(t.route, [(3, true), (5, true)]);
        assert!(near(t.tail, 1.0), "{t:?}");
    }

    #[test]
    fn dead_end_stops_head_short_by_margin() {
        let l = line();
        let t = Train::new(&l, (4, true), 0.0, cars(1, 2.0)).expect("состав");
        let t = moved(t, 10.0, 1.0, &l);
        assert_eq!(t.route, [(5, true)]);
        let (head, tail) = along(&t);
        assert!(
            near(head, 24.0 - MARGIN) && near(tail, 22.0 - MARGIN),
            "{t:?}"
        );
        assert_eq!(t.speed, 0.0);
        let close = Train::new(&l, (5, true), 1.8, cars(1, 2.0)).expect("состав");
        let t = moved(close.clone(), 1.0, 1.0, &l);
        assert_eq!((t.head, t.speed), (close.head, 0.0));
        let t = moved(close, -1.0, 1.0, &l);
        assert!(near(t.head, 2.8), "{t:?}");
        assert_eq!(t.speed, -1.0);
    }

    #[test]
    fn trailing_switch_on_other_branch_stops_forward_and_back() {
        let mut f = fork();
        f.switches[0].active = 1;
        let t = Train::new(&f, (4, false), 0.0, cars(1, 2.0)).expect("состав");
        let t = moved(t, 10.0, 1.0, &f);
        assert_eq!(t.route, [(2, false)]);
        assert!(near(t.head, 4.0 - MARGIN) && t.speed == 0.0, "{t:?}");
        f.switches[0].active = 0;
        let t = moved(t, 10.0, 0.1, &f);
        assert_eq!(t.route, [(2, false), (1, true)]);
        assert!(near(t.head, MARGIN), "{t:?}");

        f.switches[0].active = 1;
        let t = Train::new(&f, (2, true), 1.0, cars(1, 2.0)).expect("состав");
        let t = moved(t, -5.0, 1.0, &f);
        assert_eq!(t.route, [(2, true)]);
        assert!(near(t.tail, MARGIN) && t.speed == 0.0, "{t:?}");
        f.switches[0].active = 0;
        let t = moved(t, -1.0, 1.0, &f);
        assert_eq!(t.route, [(1, false), (2, true)]);
        assert!(near(t.tail, 4.0 - MARGIN), "{t:?}");
    }

    #[test]
    fn last_move_direction_outlives_stop() {
        let l = line();
        let mut t = Train::new(&l, (2, true), 0.0, cars(1, 2.0)).expect("состав");
        assert!(!t.reverse);
        t = moved(t, -1.0, DT, &l);
        assert!(t.reverse);
        t = moved(t, 0.0, DT, &l);
        assert!(t.reverse);
        t = moved(t, 1.0, DT, &l);
        assert!(!t.reverse);
    }

    #[test]
    fn standing_train_stays_put() {
        let f = fork();
        let t = Train::new(&f, (3, true), 0.1, cars(1, 2.0)).expect("состав");
        assert_eq!(moved(t.clone(), 0.0, 1.0, &f), t);
    }

    #[test]
    fn gas_accelerates_slower_with_cars_up_to_max_speed() {
        let l = line();
        let mut one = Train::new(&l, (0, true), 0.0, cars(1, 2.0)).expect("состав");
        let mut three = Train::new(&l, (0, true), 0.0, cars(3, 1.0)).expect("состав");
        one.drive(1.0, DT);
        three.drive(1.0, DT);
        assert!(near(one.speed, ACCEL * DT), "{}", one.speed);
        assert!(near(three.speed, ACCEL * DT / 3.0), "{}", three.speed);
        one.drive(0.5, DT);
        assert!(near(one.speed, 1.5 * ACCEL * DT), "{}", one.speed);
        for _ in 0..1000 {
            one.drive(1.0, DT);
            three.drive(-1.0, DT);
        }
        assert_eq!((one.speed, three.speed), (MAX_SPEED, -MAX_SPEED));
    }

    #[test]
    fn pedal_against_motion_brakes_to_stop_then_reverses() {
        let l = line();
        let mut t = Train::new(&l, (0, true), 0.0, cars(1, 2.0)).expect("состав");
        t.speed = 2.0;
        t.drive(-1.0, DT);
        assert!(near(t.speed, 2.0 - BRAKE * DT), "{}", t.speed);
        t.drive(-0.5, DT);
        assert!(near(t.speed, 2.0 - 1.5 * BRAKE * DT), "{}", t.speed);
        t.speed = 0.01;
        t.drive(-1.0, DT);
        assert_eq!(t.speed, 0.0);
        t.drive(-1.0, DT);
        assert!(near(t.speed, -ACCEL * DT), "{}", t.speed);
        t.speed = -2.0;
        t.drive(1.0, DT);
        assert!(near(t.speed, -2.0 + BRAKE * DT), "{}", t.speed);
    }

    #[test]
    fn coasting_slows_to_stop() {
        let l = line();
        let mut t = Train::new(&l, (0, true), 0.0, cars(1, 2.0)).expect("состав");
        t.speed = 1.0;
        t.drive(0.0, DT);
        assert!(near(t.speed, 1.0 - ROLLING * DT), "{}", t.speed);
        t.speed = -1.0;
        t.drive(0.0, DT);
        assert!(near(t.speed, -1.0 + ROLLING * DT), "{}", t.speed);
        t.speed = 0.001;
        t.drive(0.0, DT);
        assert_eq!(t.speed, 0.0);
    }

    #[test]
    fn cars_stand_on_axis_at_bogie_offsets() {
        let mut assets = Assets::default();
        let loco = consist::load(&mut assets, "train/train-locomotive-a");
        let wagon = consist::load(&mut assets, "train/train-carriage-tank");
        let l = line();
        let len = consist::len(&[loco, wagon]);
        let cases = [
            ((0, true), vec2(0.0, 1.0 + len), Vec2::Y),
            ((5, false), vec2(0.0, 23.0 - len), Vec2::NEG_Y),
        ];
        for (start, head, dir) in cases {
            let t = Train::new(&l, start, 1.0, vec![loco, wagon]).expect("состав");
            let (placed, couplers) = t.cars_at(&l);
            let mut back = 0.0;
            for (p, c) in placed.iter().zip([loco, wagon]) {
                for (b, off) in p.bogies.iter().zip(c.bogies) {
                    assert!(near2(b.0, head - dir * (back + off)), "{p:?}");
                    assert!(near2(b.1, dir), "{p:?}");
                }
                assert!(near2(p.pos, head - dir * (back + c.front)), "{p:?}");
                assert!(near2(p.dir, dir), "{p:?}");
                back += c.length + GAP;
            }
            let [[from, to]] = couplers[..] else {
                panic!("{couplers:?}")
            };
            assert!(near2(from, head - dir * loco.length), "{from}");
            assert!(near2(to, head - dir * (loco.length + GAP)), "{to}");
        }
    }

    #[test]
    fn body_follows_chord_between_bogies_on_corner() {
        let wagon = consist::load(&mut Assets::default(), "train/train-carriage-box");
        let c = track(&[("straight", [0, 0], 0), ("corner-large", [0, 2], 0)]);
        let tail = 5.0 - wagon.length;
        let t = Train::new(&c, (0, true), tail, vec![wagon]).expect("состав");
        assert_eq!(t.route, [(0, true), (1, true)]);
        let (placed, couplers) = t.cars_at(&c);
        let [p] = placed[..] else {
            panic!("{placed:?}")
        };
        assert!(couplers.is_empty(), "{couplers:?}");
        let want = [
            c.edges[1].rail.point_at(1.0 - wagon.bogies[0]),
            c.edges[0].rail.point_at(5.0 - wagon.bogies[1]),
        ];
        for (b, w) in p.bogies.iter().zip(want) {
            assert!(near2(b.0, w.0) && near2(b.1, w.1), "{b:?} {w:?}");
        }
        let [front, back] = p.bogies.map(|b| b.0);
        assert!(near2(p.dir, (front - back).normalize()), "{p:?}");
        assert!(
            near2(p.pos + p.dir * (wagon.front - wagon.bogies[0]), front),
            "{p:?}"
        );
    }

    /// Четыре угла габаритного прямоугольника машины на пути.
    fn corners(p: &Placed, [min, max]: [Vec2; 2]) -> [Vec2; 4] {
        let right = vec2(p.dir.y, -p.dir.x);
        [
            (min.x, min.y),
            (max.x, min.y),
            (max.x, max.y),
            (min.x, max.y),
        ]
        .map(|(x, z)| p.pos + right * x + p.dir * z)
    }

    /// Выпуклые многоугольники разделены осью одной из сторон.
    fn apart(a: &[Vec2; 4], b: &[Vec2; 4]) -> bool {
        [a, b].iter().any(|poly| {
            (0..4).any(|i| {
                let n = (poly[(i + 1) % 4] - poly[i]).perp();
                let range = |q: &[Vec2; 4]| {
                    let d = q.map(|v| v.dot(n));
                    (
                        d.into_iter().fold(f32::MAX, f32::min),
                        d.into_iter().fold(f32::MIN, f32::max),
                    )
                };
                let (a0, a1) = range(a);
                let (b0, b1) = range(b);
                a1 <= b0 + 1e-4 || b1 <= a0 + 1e-4
            })
        })
    }

    /// Каждая пара машин игры едет от `start` до остановки; `Err` — первое касание кузовов, зазор меньше на `slack`.
    fn corridor(t: &Track, start: Pass, slack: f32) -> Result<(), String> {
        let catalog = catalog();
        for (a, ra) in &catalog {
            for &(b, [min, max]) in &catalog {
                let rb = [min, max + vec2(0.0, slack)];
                let mut train = Train::new(t, start, 0.0, vec![*a, b]).expect("состав");
                for tick in 0.. {
                    assert!(tick < 3000, "не доехал: {train:?}");
                    let (p, _) = train.cars_at(t);
                    if !apart(&corners(&p[0], *ra), &corners(&p[1], rb)) {
                        return Err(format!("{a:?} {b:?} {train:?}"));
                    }
                    train.speed = MAX_SPEED;
                    train.advance(DT, t);
                    if train.speed == 0.0 {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Прямые, `corner-large`, прямые, S-деталь и зеркальная S-деталь.
    fn bends() -> Track {
        mirrored(&[
            ("straight", [0, -4], 0, false),
            ("straight", [0, -2], 0, false),
            ("corner-large", [0, 0], 0, false),
            ("straight", [-2, 2], 270, false),
            ("straight", [-4, 2], 270, false),
            ("curve", [-6, 2], 270, false),
            ("straight", [-8, 1], 270, false),
            ("curve", [-10, 1], 270, true),
            ("straight", [-12, 2], 270, false),
            ("straight", [-14, 2], 270, false),
        ])
    }

    fn sharp() -> Track {
        track(&[
            ("straight", [0, -4], 0),
            ("straight", [0, -2], 0),
            ("corner-small", [0, 0], 0),
            ("straight", [-1, 1], 270),
            ("straight", [-3, 1], 270),
        ])
    }

    #[test]
    fn neighbour_bodies_never_touch_on_level01() {
        let mut t = Track::parse(LEVEL01, 0.0).expect("level01");
        for (a, c, b) in [
            (false, false, false),
            (true, false, true),
            (true, true, false),
        ] {
            set(&mut t, A, a);
            set(&mut t, C, c);
            set(&mut t, B, b);
            assert_eq!(corridor(&t, t.depot[0], 0.0), Ok(()), "A {a}, C {c}, B {b}");
        }
        set(&mut t, A, false);
        assert!(corridor(&t, t.depot[0], 0.05).is_err(), "GAP не наименьший");
    }

    #[test]
    fn neighbour_bodies_never_touch_on_bends_both_ways() {
        let t = bends();
        let n = t.edges.len();
        assert!((1..n).all(|i| t.next((i - 1, true)) == Some((i, true))));
        assert_eq!(corridor(&t, (0, true), 0.0), Ok(()));
        assert_eq!(corridor(&t, (n - 1, false), 0.0), Ok(()));
    }

    #[test]
    fn corner_small_is_too_sharp_for_gap_and_not_on_map() {
        assert!(corridor(&sharp(), (0, true), 0.0).is_err());
        let level = Track::parse(LEVEL01, 0.0).expect("level01");
        assert!(
            level
                .edges
                .iter()
                .all(|e| e.rail.piece != Piece::CornerSmall)
        );
    }
}
