use std::time::Duration;

/// Фиксированный шаг симуляции с накопителем времени кадра.
pub struct FixedClock {
    step: Duration,
    max_frame: Duration,
    accumulator: Duration,
}

impl FixedClock {
    /// Тик длиной `1 / tick_hz`; кадр длиннее `max_frame` засчитывается как `max_frame`.
    pub fn new(tick_hz: u32, max_frame: Duration) -> Self {
        Self {
            step: Duration::from_secs_f64(1.0 / f64::from(tick_hz)),
            max_frame,
            accumulator: Duration::ZERO,
        }
    }

    /// Длина тика.
    pub fn step(&self) -> Duration {
        self.step
    }

    /// Сколько тиков выполнить за кадр длиной `frame_dt`.
    pub fn advance(&mut self, frame_dt: Duration) -> u32 {
        self.accumulator += frame_dt.min(self.max_frame);
        let mut ticks = 0;
        while let Some(rest) = self.accumulator.checked_sub(self.step) {
            self.accumulator = rest;
            ticks += 1;
        }
        ticks
    }

    /// Доля шага, прошедшая после последнего тика, для интерполяции.
    pub fn alpha(&self) -> f32 {
        self.accumulator.as_secs_f32() / self.step.as_secs_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_FRAME: Duration = Duration::from_millis(250);

    fn clock() -> FixedClock {
        FixedClock::new(60, MAX_FRAME)
    }

    #[test]
    fn one_step_gives_one_tick() {
        let mut c = clock();
        assert_eq!(c.advance(c.step()), 1);
    }

    #[test]
    fn partial_frames_accumulate() {
        let mut c = clock();
        let a = c.step() / 3;
        assert_eq!(c.advance(a), 0);
        assert_eq!(c.advance(c.step().checked_sub(a).unwrap()), 1);
        assert_eq!(c.advance(Duration::ZERO), 0);
    }

    #[test]
    fn long_frame_is_clamped_and_keeps_remainder() {
        let mut c = clock();
        let step = c.step();
        let expected = u32::try_from(MAX_FRAME.as_nanos() / step.as_nanos()).unwrap();
        assert_eq!(c.advance(Duration::from_secs(1)), expected);
        let remainder = MAX_FRAME.checked_sub(step * expected).unwrap();
        assert!(remainder > Duration::ZERO);
        let rest = step
            .checked_sub(remainder + Duration::from_nanos(1))
            .unwrap();
        assert_eq!(c.advance(rest), 0);
        assert_eq!(c.advance(Duration::from_nanos(1)), 1);
    }

    #[test]
    fn alpha_stays_in_unit_range() {
        let mut c = clock();
        assert_eq!(c.alpha(), 0.0);
        for ms in [1, 5, 7, 16, 17, 33, 100, 250, 1000] {
            c.advance(Duration::from_millis(ms));
            let alpha = c.alpha();
            assert!((0.0..1.0).contains(&alpha), "alpha = {alpha}");
        }
        let mut half = clock();
        half.advance(half.step() / 2);
        assert!((half.alpha() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn tick_hz_sets_step() {
        let c = FixedClock::new(30, MAX_FRAME);
        assert!((c.step().as_secs_f64() - 1.0 / 30.0).abs() < 1e-9);
    }
}
