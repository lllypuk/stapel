//! Автопилот тестов: руль на точку средней линии впереди, газ по кривизне.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use engine::{Context, Game, Vec2, Vec3};

use crate::car::{Car, Controls, steer_angle};
use crate::garage::GarageCmd;
use crate::race::lap_time;
use crate::track::{Checkpoint, TrackDraft, cell_at};
use crate::{CARS_DIR, Racing, TICK_HZ, TRACKS_DIR, load_cars};

/// Точек средней линии на деталь.
const SAMPLES: usize = 16;
/// Ручки Безье — доля хорды: так кривая повторяет четверть окружности.
const HANDLE: f32 = 0.39;
/// Боковое ускорение в поворотах, м/с².
const LATERAL: f32 = 6.0;
/// Замедление тормозом в расчёте скорости, м/с².
const DECEL: f32 = 3.0;
/// Сколько пути вперёд смотрит газ, м.
const HORIZON: f32 = 60.0;
/// Колёсная база для руля, м.
const WHEELBASE: f32 = 1.8;
/// Ближайшая точка ищется вперёд от прежней: на восьмёрке линия пересекает себя.
const SEARCH: usize = 2 * SAMPLES;

pub(crate) struct Autopilot {
    /// Средняя линия по XZ, замкнутая.
    path: Vec<Vec2>,
    /// Предельная скорость в точках по кривизне, м/с.
    limit: Vec<f32>,
    /// Длина отрезка от точки к следующей, м.
    step: Vec<f32>,
    /// Ближайшая точка линии к машине.
    at: usize,
}

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

impl Autopilot {
    /// Линия по чекпоинтам `track.chain()`: кубика Безье от входа детали к выходу.
    pub(crate) fn new(checkpoints: &[Checkpoint]) -> Self {
        let pieces = checkpoints.len();
        let mut path = Vec::with_capacity(pieces * SAMPLES);
        for (i, exit) in checkpoints.iter().enumerate() {
            let entry = &checkpoints[(i + pieces - 1) % pieces];
            let (p0, p3) = (xz(entry.origin), xz(exit.origin));
            let k = HANDLE * p0.distance(p3);
            let p1 = p0 + xz(entry.normal) * k;
            let p2 = p3 - xz(exit.normal) * k;
            path.extend((0..SAMPLES).map(|s| {
                let t = s as f32 / SAMPLES as f32;
                let u = 1.0 - t;
                p0 * u * u * u + p1 * 3.0 * u * u * t + p2 * 3.0 * u * t * t + p3 * t * t * t
            }));
        }
        let m = path.len();
        let step: Vec<f32> = (0..m)
            .map(|j| path[j].distance(path[(j + 1) % m]))
            .collect();
        let limit = (0..m)
            .map(|j| {
                let (a, b, c) = (path[(j + m - 1) % m], path[j], path[(j + 1) % m]);
                let turn = (b - a).angle_to(c - b).abs();
                let len = f32::midpoint(step[(j + m - 1) % m], step[j]);
                (LATERAL * len / turn.max(1e-6)).sqrt()
            })
            .collect();
        Self {
            path,
            limit,
            step,
            at: 0,
        }
    }

    pub(crate) fn controls(&mut self, car: &Car) -> Controls {
        let m = self.path.len();
        let pos = xz(car.body.pos);
        self.at = (self.at..=self.at + SEARCH)
            .map(|j| j % m)
            .min_by(|&a, &b| {
                let (da, db) = (self.path[a].distance(pos), self.path[b].distance(pos));
                da.total_cmp(&db)
            })
            .expect("окно поиска не пусто");
        let speed = car.forward_speed();
        let (lookahead, mut allowed) = (4.0 + 0.3 * speed.abs(), f32::INFINITY);
        let (mut s, mut target) = (0.0, None);
        for k in 0..m {
            let j = (self.at + k) % m;
            if target.is_none() && s >= lookahead {
                target = Some(self.path[j]);
            }
            allowed = allowed.min((self.limit[j].powi(2) + 2.0 * DECEL * s).sqrt());
            s += self.step[j];
            if s > HORIZON {
                break;
            }
        }
        let to = target.expect("линия длиннее взгляда вперёд") - pos;
        let fwd = xz(car.body.rot * Vec3::Z);
        let alpha = to.perp_dot(fwd).atan2(to.dot(fwd));
        let wheel = (2.0 * WHEELBASE * alpha.sin() / to.length()).atan();
        let full = steer_angle(&car.config, 1.0, speed);
        Controls {
            throttle: f32::from(u8::from(speed < allowed)),
            brake: f32::from(u8::from(speed > allowed + 0.5)),
            steer: (wheel / full).clamp(-1.0, 1.0),
            handbrake: false,
        }
    }
}

/// Эталон круга, с: время автопилота на машине `race`, округлённое вверх.
fn reference(track: &str) -> u64 {
    match track {
        "track01" => 10,
        "track02" => 14,
        "track03" => 17,
        _ => panic!("нет эталона {track}"),
    }
}

/// Круг машины `car` по трассе `track` не дольше 2× эталона; паника при съезде или падении.
fn lap(track: &str, car: &str) {
    let path = PathBuf::from(format!("{TRACKS_DIR}{track}.toml"));
    let text = fs::read_to_string(&path).expect("трасса");
    let parsed = TrackDraft::parse(&text).expect("разбор");
    let mut ctx = Context::default();
    let mut g = Racing::new(path, text, load_cars(CARS_DIR), &mut ctx.assets);
    let i = (g.cars.iter().position(|c| c.model == car)).expect("машина");
    g.garage.queue.extend([GarageCmd::Select(i), GarageCmd::Go]);
    g.update(&mut ctx);
    assert_eq!(g.car.config.model, car);
    let mut pilot = Autopilot::new(&g.race.checkpoints);
    let n = g.race.checkpoints.len();
    // Секунда сверху — доехать от старта до линии.
    let limit = 2 * reference(track) * u64::from(TICK_HZ);
    let clock = Instant::now();
    for t in 0..limit + u64::from(TICK_HZ) {
        g.drive(pilot.controls(&g.car));
        let body = &g.car.body;
        assert!(
            body.pos.y > 0.0,
            "{track} {car}: тик {t}, упала: {}",
            body.pos
        );
        let last = &g.race.checkpoints[(g.race.next + n - 1) % n];
        let flying = parsed.roads[last.road].piece.name == "roadRamp"
            && g.car.wheels.iter().all(|w| w.ground.is_none());
        let on_road = cell_at(body.pos).is_some_and(|c| g.road_cells.contains(&c));
        assert!(
            on_road || flying,
            "{track} {car}: тик {t}, вне дороги: {}",
            body.pos
        );
        if let Some(best) = g.race.best {
            let time = lap_time(best);
            eprintln!("{track} {car}: круг {time}, счёт {:.2?}", clock.elapsed());
            assert!(best <= limit, "{track} {car}: {time}");
            return;
        }
    }
    panic!(
        "{track} {car}: круга нет за {limit} тиков, next {}",
        g.race.next
    );
}

/// Машины по трассам так, чтобы были все приводы: `race` — RWD, `hatchback-sports` — FWD, `race-future` — AWD.
#[test]
fn race_laps_oval() {
    lap("track01", "race");
}

#[test]
fn hatchback_laps_ramp_jump() {
    lap("track02", "hatchback-sports");
}

#[test]
fn race_future_laps_bridge_eight() {
    lap("track03", "race-future");
}

/// Каждая машина на каждой трассе; вне мутантов — каждый мутант гонял бы 12 кругов.
#[test]
#[ignore = "долгий: make test и make cov включают"]
fn every_car_laps_every_track() {
    let mut tracks: Vec<String> = fs::read_dir(TRACKS_DIR)
        .expect("трассы")
        .map(|e| {
            let path = e.expect("трасса").path();
            path.file_stem()
                .expect("имя")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    tracks.sort();
    assert_eq!(tracks, ["track01", "track02", "track03"]);
    for track in &tracks {
        for car in load_cars(CARS_DIR) {
            lap(track, &car.model);
        }
    }
}
