use std::collections::HashMap;

use stapel::{IVec2, Vec3};

use crate::TICK_HZ;
use crate::track::{Checkpoint, TrackDraft, cell_at};

/// Тиков против цепочки до «не туда»: дольше 1 с.
const WRONG_WAY_TICKS: u32 = TICK_HZ;
/// Медленнее этого против цепочки — не «не туда»: откат от барьера, разворот, м/с.
const WRONG_WAY_SPEED: f32 = 1.0;

/// Пройденный за тик чекпоинт.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pass {
    pub(crate) checkpoint: usize,
    /// Тиков законченного круга; `None` — круг не кончился.
    pub(crate) lap: Option<u64>,
}

/// Круги по чекпоинтам, время по тикам и «не туда».
pub(crate) struct Race {
    pub(crate) checkpoints: Vec<Checkpoint>,
    /// Направление цепочки в клетках дороги: от входа детали к её выходу.
    dirs: HashMap<(IVec2, u32), Vec3>,
    /// Индекс следующего чекпоинта.
    pub(crate) next: usize,
    /// Пройдена линия старта.
    pub(crate) started: bool,
    /// Законченных кругов.
    pub(crate) laps: u32,
    /// Тиков текущего круга.
    pub(crate) ticks: u64,
    pub(crate) best: Option<u64>,
    /// Тиков подряд против цепочки.
    pub(crate) wrong: u32,
}

impl Race {
    /// `checkpoints` — `track.chain()`: выход детали `i` — вход детали `i + 1`.
    pub(crate) fn new(track: &TrackDraft, checkpoints: Vec<Checkpoint>) -> Self {
        let n = checkpoints.len();
        let mut dirs = HashMap::new();
        for (i, c) in checkpoints.iter().enumerate() {
            let entry = checkpoints[(i + n - 1) % n].origin;
            let dir = (c.origin - entry).with_y(0.0).normalize_or_zero();
            dirs.extend(track.roads[c.road].cells().map(|cell| (cell, dir)));
        }
        Self {
            checkpoints,
            dirs,
            next: 0,
            started: false,
            laps: 0,
            ticks: 0,
            best: None,
            wrong: 0,
        }
    }

    /// Заезд заново; лучший круг остаётся.
    pub(crate) fn restart(&mut self) {
        self.next = 0;
        self.started = false;
        self.laps = 0;
        self.ticks = 0;
        self.wrong = 0;
    }

    /// Тик центра масс `from → to` со скоростью `vel`.
    pub(crate) fn tick(&mut self, from: Vec3, to: Vec3, vel: Vec3) -> Option<Pass> {
        let dir = cell_at(to).and_then(|c| self.dirs.get(&c));
        let against = dir.is_some_and(|d| vel.dot(*d) < -WRONG_WAY_SPEED);
        self.wrong = if against { self.wrong + 1 } else { 0 };
        if self.started {
            self.ticks += 1;
        }
        let i = self.next;
        if !self.checkpoints[i].crossed(from, to) {
            return None;
        }
        let mut lap = None;
        if i == 0 {
            if self.started {
                self.laps += 1;
                self.best = Some(self.best.map_or(self.ticks, |b| b.min(self.ticks)));
                lap = Some(self.ticks);
            }
            self.started = true;
            self.ticks = 0;
        }
        self.next = (i + 1) % self.checkpoints.len();
        Some(Pass { checkpoint: i, lap })
    }

    pub(crate) fn wrong_way(&self) -> bool {
        self.wrong > WRONG_WAY_TICKS
    }
}

/// `m:ss.cc`, сотые отбрасываются.
pub(crate) fn lap_time(ticks: u64) -> String {
    let cs = ticks * 100 / u64::from(TICK_HZ);
    format!("{}:{:02}.{:02}", cs / 6000, cs / 100 % 60, cs % 100)
}

#[cfg(test)]
mod tests {
    use stapel::vec3;

    use super::*;
    use crate::pieces::KIT_SCALE;

    fn oval() -> Race {
        let track = TrackDraft::parse(include_str!("testdata/oval.toml")).expect("овал");
        let checkpoints = track.chain().expect("цепочка");
        Race::new(&track, checkpoints)
    }

    /// Тик через чекпоинт `i` по направлению цепочки.
    fn pass_through(r: &mut Race, i: usize) -> Option<Pass> {
        let (at, n) = (
            r.checkpoints[i].origin + Vec3::Y * 0.5,
            r.checkpoints[i].normal,
        );
        r.tick(at - n * 0.5, at + n * 0.5, n * 30.0)
    }

    fn cross(r: &mut Race, i: usize) -> Option<usize> {
        pass_through(r, i).map(|p| p.checkpoint)
    }

    /// `n` тиков стоя у старта.
    fn idle(r: &mut Race, n: u64) {
        let at = r.checkpoints[0].origin - r.checkpoints[0].normal + Vec3::Y * 0.5;
        for _ in 0..n {
            assert_eq!(r.tick(at, at, Vec3::ZERO), None);
        }
    }

    /// Круг от старта до старта: `ticks` тиков между пересечениями линии.
    fn lap(r: &mut Race, ticks: u64) {
        let n = r.checkpoints.len();
        idle(r, ticks - n as u64);
        for i in 1..n {
            assert_eq!(cross(r, i), Some(i));
        }
        assert_eq!(cross(r, 0), Some(0));
    }

    #[test]
    fn lap_counts_only_in_checkpoint_order() {
        let mut r = oval();
        let n = r.checkpoints.len();
        assert!(n > 3, "{n}");
        assert_eq!(cross(&mut r, 1), None, "до старта");
        assert_eq!(cross(&mut r, 0), Some(0));
        assert!(r.started);
        assert_eq!((r.next, r.laps), (1, 0));
        assert_eq!(cross(&mut r, 2), None);
        assert_eq!(cross(&mut r, 0), None);
        assert_eq!(r.next, 1);
        for i in 1..n {
            assert_eq!(cross(&mut r, i), Some(i));
            assert_eq!(r.laps, 0);
        }
        assert_eq!(cross(&mut r, 0), Some(0));
        assert_eq!((r.next, r.laps), (1, 1));
    }

    #[test]
    fn cut_past_checkpoint_does_not_count() {
        let mut r = oval();
        let n = r.checkpoints.len();
        cross(&mut r, 0);
        for i in (1..n).filter(|&i| i != 2) {
            cross(&mut r, i);
        }
        assert_eq!(cross(&mut r, 0), None);
        assert_eq!((r.next, r.laps, r.best), (2, 0, None));
    }

    #[test]
    fn crossing_backwards_does_not_count() {
        let mut r = oval();
        let (at, n) = (
            r.checkpoints[0].origin + Vec3::Y * 0.5,
            r.checkpoints[0].normal,
        );
        assert_eq!(r.tick(at + n * 0.5, at - n * 0.5, -n), None);
        assert!(!r.started);
    }

    #[test]
    fn lap_time_counts_ticks_between_start_crossings() {
        let mut r = oval();
        idle(&mut r, 50);
        assert_eq!(r.ticks, 0, "до старта время стоит");
        cross(&mut r, 0);
        assert_eq!(r.ticks, 0);
        idle(&mut r, 7);
        assert_eq!(r.ticks, 7);
        lap(&mut r, 300 - 7);
        assert_eq!((r.laps, r.best, r.ticks), (1, Some(300), 0));
    }

    #[test]
    fn tick_reports_finished_lap_at_start_line_only() {
        let mut r = oval();
        let pass = |checkpoint, lap| Some(Pass { checkpoint, lap });
        assert_eq!(
            pass_through(&mut r, 0),
            pass(0, None),
            "старт — не конец круга"
        );
        let n = r.checkpoints.len();
        idle(&mut r, 300 - n as u64);
        for i in 1..n {
            assert_eq!(pass_through(&mut r, i), pass(i, None));
        }
        assert_eq!(pass_through(&mut r, 0), pass(0, Some(300)));
    }

    #[test]
    fn best_improves_only_by_smaller_lap() {
        let mut r = oval();
        cross(&mut r, 0);
        lap(&mut r, 300);
        lap(&mut r, 400);
        assert_eq!((r.laps, r.best), (2, Some(300)));
        lap(&mut r, 200);
        assert_eq!((r.laps, r.best), (3, Some(200)));
    }

    #[test]
    fn restart_keeps_best_only() {
        let mut r = oval();
        cross(&mut r, 0);
        lap(&mut r, 300);
        cross(&mut r, 1);
        idle(&mut r, 5);
        r.wrong = 3;
        r.restart();
        assert_eq!((r.next, r.started, r.laps), (0, false, 0));
        assert_eq!((r.ticks, r.wrong, r.best), (0, 0, Some(300)));
    }

    #[test]
    fn wrong_way_after_one_second_against_chain() {
        let mut r = oval();
        let n = r.checkpoints.len();
        let i = (1..n)
            .find(|&i| r.checkpoints[i].normal == r.checkpoints[i - 1].normal)
            .expect("прямая");
        let dir = r.checkpoints[i].normal;
        let at = r.checkpoints[i].origin - dir + Vec3::Y * 0.5;
        let back = -dir * 5.0;
        for t in 1..=TICK_HZ {
            r.tick(at, at, back);
            assert!(!r.wrong_way(), "{t}");
        }
        r.tick(at, at, back);
        assert!(r.wrong_way());
        r.tick(at, at, dir * 5.0);
        assert!(!r.wrong_way());
        assert_eq!(r.wrong, 0);

        r.tick(at, at, -dir * WRONG_WAY_SPEED);
        assert_eq!(r.wrong, 0, "медленный откат");
        r.tick(at, at, back + dir.cross(Vec3::Y) * 40.0);
        assert_eq!(r.wrong, 1, "против цепочки и вбок");
        // [3, 2] — внутри овала, не дорога.
        let inside = vec3(3.5, 0.1, 2.5) * KIT_SCALE;
        r.tick(inside, inside, back);
        assert_eq!(r.wrong, 0);
    }

    #[test]
    fn wrong_way_follows_piece_under_car_not_next_checkpoint() {
        let mut r = oval();
        cross(&mut r, 0);
        let n = r.checkpoints.len();
        let far = (0..n)
            .find(|&i| r.checkpoints[i].normal.dot(r.checkpoints[0].normal) < -0.9)
            .expect("обратная сторона овала");
        let dir = r.checkpoints[far].normal;
        let at = r.checkpoints[far].origin - dir + Vec3::Y * 0.5;
        assert!((r.checkpoints[1].origin - at).dot(dir) < 0.0);
        for _ in 0..2 * TICK_HZ {
            r.tick(at, at, dir * 20.0);
        }
        assert_eq!(r.wrong, 0);
        for _ in 0..2 * TICK_HZ {
            r.tick(at, at, -dir * 20.0);
        }
        assert!(r.wrong_way());
    }

    #[test]
    fn chain_direction_follows_corner_from_entry_to_exit() {
        let r = oval();
        let n = r.checkpoints.len();
        let i = (0..n)
            .find(|&i| r.checkpoints[i].normal != r.checkpoints[(i + n - 1) % n].normal)
            .expect("поворот");
        let (entry, exit) = (
            r.checkpoints[(i + n - 1) % n].normal,
            r.checkpoints[i].normal,
        );
        let cell = cell_at(r.checkpoints[i].origin - exit * 0.5 + Vec3::Y * 0.5).expect("клетка");
        let dir = r.dirs[&cell];
        assert!(
            dir.dot(entry) > 0.5 && dir.dot(exit) > 0.5,
            "{dir} {entry} {exit}"
        );
    }

    #[test]
    fn lap_time_formats_minutes_seconds_hundredths() {
        assert_eq!(lap_time(0), "0:00.00");
        assert_eq!(lap_time(59), "0:00.98");
        assert_eq!(lap_time(u64::from(TICK_HZ) * 61 + 30), "1:01.50");
        assert_eq!(lap_time(u64::from(TICK_HZ) * 600), "10:00.00");
    }
}
