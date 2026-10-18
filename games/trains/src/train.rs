use std::collections::VecDeque;

use crate::consist::{self, Car};
use crate::track::{MARGIN, Pass, Track};

/// Предел скорости в обе стороны, м/с.
pub(crate) const MAX_SPEED: f32 = 6.0;
/// Разгон одной машины, м/с²; с вагонами делится на их число.
const ACCEL: f32 = 3.0;
/// Торможение педалью против хода, м/с².
const BRAKE: f32 = 4.0;
/// Замедление накатом без педали, м/с².
const ROLLING: f32 = 0.3;

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
        Some(Self {
            route,
            head,
            tail,
            speed: 0.0,
            cars,
        })
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
        loop {
            let len = track.edges[self.route[0].0].len();
            if self.tail <= len {
                break;
            }
            self.tail -= len;
            self.route.pop_front();
        }
        blocked
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

    use engine::Assets;

    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn track(rails: &[(&str, [i32; 2], i64)]) -> Track {
        let mut text = String::from("depot = { rail = 0 }\n");
        for (piece, [x, z], rot) in rails {
            writeln!(
                text,
                "[[rail]]\npiece = \"{piece}\"\ncell = [{x}, {z}]\nrot = {rot}"
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
        let mut assets = Assets::default();
        let h = assets.load_model(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/train/train-carriage-box.glb"
        ));
        let car = Car::new(h, assets.model(h)).expect("вагон");
        vec![Car { length, ..car }; n]
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
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
        assert!(near(t.head, 3.6) && near(t.tail, 1.0), "{t:?}");
        assert_eq!(t.speed, 0.0);
        let t = Train::new(&l, (0, true), 2.0, cars(1, 2.0)).expect("состав");
        assert_eq!((t.route, t.head), ([(0, true)].into(), 4.0));
        assert_eq!(Train::new(&l, (5, true), 3.0, cars(2, 2.0)), None);
        let mut f = fork();
        f.switches[0].active = 1;
        let t = Train::new(&f, (0, true), 1.0, cars(4, 2.0)).expect("состав");
        assert_eq!(t.route, [(0, true), (1, false), (3, true)]);
        assert!(near(t.head, 1.9), "{t:?}");
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
            (&[(3, true), (4, true)].into(), 2.0, 4.0)
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
}
