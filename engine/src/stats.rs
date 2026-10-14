use std::time::{Duration, Instant};

use crate::app::MAX_FRAME;

const WINDOW: Duration = Duration::from_secs(1);

/// Округлённое число событий в секунду.
fn per_second(count: u32, elapsed: Duration) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "elapsed ≥ WINDOW = 1 с, значит частота в [0, count]"
    )]
    let rate = (f64::from(count) / elapsed.as_secs_f64()).round() as u32;
    rate
}

/// Счётчик событий в секунду (FPS, TPS).
#[derive(Default)]
pub struct RateCounter {
    window_start: Option<Instant>,
    count: u32,
    report: Option<u32>,
}

impl RateCounter {
    /// Счётчик без событий.
    pub fn new() -> Self {
        Self::default()
    }

    /// Событие закрывает прошедшую секунду и уходит в счёт следующей.
    pub fn record(&mut self, now: Instant) {
        let start = *self.window_start.get_or_insert(now);
        let elapsed = now.duration_since(start);
        if elapsed >= WINDOW {
            self.report = Some(per_second(self.count, elapsed));
            self.count = 0;
            self.window_start = Some(now);
        }
        self.count += 1;
    }

    /// Пауза: открытое окно отбрасывается, иначе частота после неё занижена.
    pub fn reset(&mut self) {
        self.window_start = None;
        self.count = 0;
    }

    /// Частота за последнее закрытое окно замера (не короче секунды); отдаётся один раз.
    pub fn per_second(&mut self) -> Option<u32> {
        self.report.take()
    }
}

/// Итог окна кадров: интервалы между успешными `render` и спрайты тех же кадров.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameReport {
    /// Кадров в секунду.
    pub fps: u32,
    /// Самый длинный интервал между кадрами.
    pub worst: Duration,
    /// Наименьшее число спрайтов в кадре.
    pub min_sprites: usize,
    /// Наибольшее число спрайтов в кадре.
    pub max_sprites: usize,
}

/// Статистика кадров по окнам не короче секунды; окна режутся по концу интервала, как у `RateCounter`.
#[derive(Default)]
pub struct FrameStats {
    last: Option<Instant>,
    window_start: Option<Instant>,
    count: u32,
    worst: Duration,
    min_sprites: usize,
    max_sprites: usize,
    report: Option<FrameReport>,
}

impl FrameStats {
    /// Статистика без кадров.
    pub fn new() -> Self {
        Self::default()
    }

    /// `now` — конец успешного `render`, `sprites` — спрайты этого кадра; после `reset` только ставит отметку.
    pub fn record(&mut self, now: Instant, sprites: usize) {
        let Some(last) = self.last.replace(now) else {
            return;
        };
        let interval = now.duration_since(last);
        if interval > MAX_FRAME {
            self.reset();
            self.last = Some(now);
            return;
        }
        let start = *self.window_start.get_or_insert(now);
        let elapsed = now.duration_since(start);
        if elapsed >= WINDOW {
            self.report = Some(FrameReport {
                fps: per_second(self.count, elapsed),
                worst: self.worst,
                min_sprites: self.min_sprites,
                max_sprites: self.max_sprites,
            });
            self.count = 0;
            self.window_start = Some(now);
        }
        if self.count == 0 {
            self.worst = interval;
            self.min_sprites = sprites;
            self.max_sprites = sprites;
        } else {
            self.worst = self.worst.max(interval);
            self.min_sprites = self.min_sprites.min(sprites);
            self.max_sprites = self.max_sprites.max(sprites);
        }
        self.count += 1;
    }

    /// Пауза в рендере: открытое окно отбрасывается, иначе его fps и `worst` включат паузу.
    /// Интервал длиннее `MAX_FRAME` — тоже пауза: свёрнутое окно на Wayland не даёт `Ok(false)`.
    pub fn reset(&mut self) {
        *self = Self {
            report: self.report.take(),
            ..Self::default()
        };
    }

    /// Итог последнего закрытого окна; отдаётся один раз.
    pub fn report(&mut self) -> Option<FrameReport> {
        self.report.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_60(c: &mut RateCounter, from: Instant) {
        for i in 0..60 {
            c.record(from + WINDOW / 60 * i);
        }
    }

    #[test]
    fn sixty_events_in_one_second() {
        let t0 = Instant::now();
        let mut c = RateCounter::new();
        record_60(&mut c, t0);
        c.record(t0 + WINDOW);
        assert_eq!(c.per_second(), Some(60));
    }

    #[test]
    fn reset_discards_open_window() {
        let t0 = Instant::now();
        let mut c = RateCounter::new();
        for i in 0..30 {
            c.record(t0 + Duration::from_secs(1) * i / 60);
        }
        c.reset();
        let t1 = t0 + Duration::from_secs(10);
        record_60(&mut c, t1);
        assert_eq!(c.per_second(), None);
        c.record(t1 + WINDOW);
        assert_eq!(c.per_second(), Some(60));
    }

    #[test]
    fn longer_window_is_scaled_to_one_second() {
        let t0 = Instant::now();
        let mut c = RateCounter::new();
        for i in 0..=5 {
            c.record(t0 + Duration::from_millis(240) * i);
        }
        assert_eq!(c.per_second(), Some(4));
    }

    #[test]
    fn no_report_before_second_ends() {
        let t0 = Instant::now();
        let mut c = RateCounter::new();
        record_60(&mut c, t0);
        assert_eq!(c.per_second(), None);
    }

    #[test]
    fn count_restarts_after_report() {
        let t0 = Instant::now();
        let mut c = RateCounter::new();
        record_60(&mut c, t0);
        let t1 = t0 + WINDOW;
        for i in 0..30 {
            c.record(t1 + WINDOW / 30 * i);
        }
        assert_eq!(c.per_second(), Some(60));
        assert_eq!(c.per_second(), None);
        c.record(t1 + WINDOW);
        assert_eq!(c.per_second(), Some(30));
    }

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Отметка в `from` и `n` кадров через 10 мс со спрайтами `sprites`.
    fn frames(s: &mut FrameStats, t0: Instant, from: u64, n: u64, sprites: usize) -> u64 {
        for i in 1..=n {
            s.record(ms(t0, from + 10 * i), sprites);
        }
        from + 10 * n
    }

    #[test]
    fn frame_window_is_at_least_one_second() {
        let t0 = Instant::now();
        let mut s = FrameStats::new();
        s.record(t0, 0);
        let end = frames(&mut s, t0, 0, 100, 1);
        assert_eq!(s.report(), None);
        s.record(ms(t0, end + 10), 1);
        let r = s.report().unwrap();
        assert_eq!((r.fps, r.worst), (100, Duration::from_millis(10)));
        assert_eq!(s.report(), None);
    }

    #[test]
    fn worst_is_longest_interval_and_crossing_one_goes_to_next_window() {
        let t0 = Instant::now();
        let mut s = FrameStats::new();
        s.record(t0, 0);
        let mut end = frames(&mut s, t0, 0, 50, 1);
        end += 40;
        s.record(ms(t0, end), 1);
        end = frames(&mut s, t0, end, 45, 1);
        end += 100;
        s.record(ms(t0, end), 1);
        let r = s.report().unwrap();
        assert_eq!(r.worst, Duration::from_millis(40));
        frames(&mut s, t0, end, 100, 1);
        assert_eq!(s.report().unwrap().worst, Duration::from_millis(100));
    }

    #[test]
    fn sprites_range_over_frames_of_window() {
        let t0 = Instant::now();
        let mut s = FrameStats::new();
        s.record(t0, 9_000);
        let mut end = frames(&mut s, t0, 0, 30, 5_200);
        end = frames(&mut s, t0, end, 1, 6_150);
        end = frames(&mut s, t0, end, 69, 5_012);
        frames(&mut s, t0, end, 1, 1);
        let r = s.report().unwrap();
        assert_eq!((r.min_sprites, r.max_sprites), (5_012, 6_150));
    }

    #[test]
    fn reset_starts_fresh_window() {
        let t0 = Instant::now();
        let mut s = FrameStats::new();
        s.record(t0, 0);
        let mut end = frames(&mut s, t0, 0, 30, 9_000);
        s.reset();
        end += 10_000;
        s.record(ms(t0, end), 1);
        end = frames(&mut s, t0, end, 100, 1);
        assert_eq!(s.report(), None);
        s.record(ms(t0, end + 10), 1);
        let r = s.report().unwrap();
        assert_eq!((r.fps, r.worst), (100, Duration::from_millis(10)));
        assert_eq!((r.min_sprites, r.max_sprites), (1, 1));
    }

    #[test]
    fn long_interval_is_pause_without_reset() {
        let t0 = Instant::now();
        let mut s = FrameStats::new();
        s.record(t0, 0);
        let mut end = frames(&mut s, t0, 0, 30, 9_000);
        end += 5_000;
        s.record(ms(t0, end), 1);
        end = frames(&mut s, t0, end, 100, 1);
        assert_eq!(s.report(), None);
        s.record(ms(t0, end + 10), 1);
        let r = s.report().unwrap();
        assert_eq!((r.fps, r.worst), (100, Duration::from_millis(10)));
        assert_eq!((r.min_sprites, r.max_sprites), (1, 1));
    }
}
