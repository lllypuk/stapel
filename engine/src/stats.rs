use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(1);

/// Счётчик событий в секунду (FPS, TPS).
#[derive(Default)]
pub struct RateCounter {
    window_start: Option<Instant>,
    count: u32,
    report: Option<u32>,
}

impl RateCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Событие закрывает прошедшую секунду и уходит в счёт следующей.
    pub fn record(&mut self, now: Instant) {
        let start = *self.window_start.get_or_insert(now);
        let elapsed = now.duration_since(start);
        if elapsed >= WINDOW {
            self.report = Some((f64::from(self.count) / elapsed.as_secs_f64()).round() as u32);
            self.count = 0;
            self.window_start = Some(now);
        }
        self.count += 1;
    }

    /// Частота за последнее закрытое окно замера (не короче секунды); отдаётся один раз.
    pub fn per_second(&mut self) -> Option<u32> {
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
}
