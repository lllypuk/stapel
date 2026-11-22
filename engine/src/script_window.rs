use std::time::{Duration, Instant};

use crate::runner::STALL_TIMEOUT;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    WaitUntil(Instant),
    Stalled,
    Advance,
    Retry(u32),
}

pub(crate) struct ScriptWindow {
    period: Duration,
    next_frame: Instant,
    last_progress: Instant,
    last_tick: u32,
    pending_tick: Option<u32>,
    available: bool,
}

impl ScriptWindow {
    pub(crate) fn new(now: Instant, tick_hz: u32, last_tick: u32, available: bool) -> Self {
        let period = Duration::from_secs_f64(1.0 / f64::from(tick_hz));
        assert!(!period.is_zero());
        Self {
            period,
            next_frame: now,
            last_progress: now,
            last_tick,
            pending_tick: None,
            available,
        }
    }

    pub(crate) fn set_available(&mut self, now: Instant, available: bool) {
        if available && !self.available {
            self.next_frame = now;
        }
        self.available = available;
    }

    /// Выдача `Advance` или `Retry` расходует текущий срок показа.
    pub(crate) fn poll(&mut self, now: Instant) -> Action {
        let stall = self.last_progress + STALL_TIMEOUT;
        if now >= stall {
            return Action::Stalled;
        }
        if !self.available {
            return Action::WaitUntil(stall);
        }
        if now < self.next_frame {
            return Action::WaitUntil(self.next_frame.min(stall));
        }

        self.next_frame += self.period;
        self.skip_expired_deadline(now);
        self.pending_tick.map_or(Action::Advance, Action::Retry)
    }

    pub(crate) fn record_advance(&mut self, tick: u32) {
        self.pending_tick = Some(tick);
    }

    /// `now` берётся после попытки показа. `true` означает, что пора завершить сценарий.
    pub(crate) fn record_present(&mut self, now: Instant, shown: bool) -> bool {
        self.skip_expired_deadline(now);
        if !shown {
            return false;
        }
        self.last_progress = now;
        self.pending_tick.take() == Some(self.last_tick)
    }

    fn skip_expired_deadline(&mut self, now: Instant) {
        if self.next_frame <= now {
            self.next_frame = now + self.period;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: Duration = Duration::from_millis(250);

    #[test]
    fn first_frame_is_due_immediately_and_early_wakes_keep_the_deadline() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 60, 1, true);
        let next = start + Duration::from_nanos(16_666_667);

        assert_eq!(schedule.poll(start), Action::Advance);
        assert_eq!(schedule.poll(start), Action::WaitUntil(next));
        schedule.record_advance(0);
        assert!(!schedule.record_present(start, true));
        assert_eq!(schedule.poll(start), Action::WaitUntil(next));
        assert_eq!(
            schedule.poll(next.checked_sub(Duration::from_nanos(1)).unwrap()),
            Action::WaitUntil(next)
        );
        assert_eq!(schedule.poll(next), Action::Advance);
        schedule.record_advance(1);
        assert!(schedule.record_present(next, true));
    }

    #[test]
    fn late_wake_skips_missed_deadlines_without_catching_up() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 2, true);

        assert_eq!(schedule.poll(start), Action::Advance);
        schedule.record_advance(0);
        assert!(!schedule.record_present(start, true));

        let late = start + 5 * PERIOD;
        assert_eq!(schedule.poll(late), Action::Advance);
        schedule.record_advance(1);
        assert!(!schedule.record_present(late, true));
        assert_eq!(schedule.poll(late), Action::WaitUntil(late + PERIOD));
        assert_eq!(schedule.poll(late + PERIOD), Action::Advance);
    }

    #[test]
    fn deadline_after_present_is_strictly_future_even_if_present_overruns() {
        let start = Instant::now();
        for elapsed in [PERIOD, 3 * PERIOD + Duration::from_millis(1)] {
            for shown in [false, true] {
                let mut schedule = ScriptWindow::new(start, 4, 2, true);
                assert_eq!(schedule.poll(start), Action::Advance);
                schedule.record_advance(0);

                let returned = start + elapsed;
                assert!(!schedule.record_present(returned, shown));
                let next = returned + PERIOD;
                assert_eq!(schedule.poll(returned), Action::WaitUntil(next));
                let expected = if shown {
                    Action::Advance
                } else {
                    Action::Retry(0)
                };
                assert_eq!(schedule.poll(next), expected);
            }
        }
    }

    #[test]
    fn failed_present_retries_same_tick_without_postponing_stall() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 2, true);
        let stall = start + STALL_TIMEOUT;

        assert_eq!(schedule.poll(start), Action::Advance);
        schedule.record_advance(0);
        assert!(!schedule.record_present(start, false));
        assert_eq!(schedule.poll(start), Action::WaitUntil(start + PERIOD));
        assert_eq!(schedule.poll(start + PERIOD), Action::Retry(0));
        assert!(!schedule.record_present(start + PERIOD, false));

        let last_retry = stall.checked_sub(PERIOD / 2).unwrap();
        assert_eq!(schedule.poll(last_retry), Action::Retry(0));
        assert!(!schedule.record_present(last_retry, false));
        assert_eq!(schedule.poll(last_retry), Action::WaitUntil(stall));
        assert_eq!(schedule.poll(stall), Action::Stalled);
    }

    #[test]
    fn successful_present_moves_stall_deadline_to_the_time_it_returned() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 2, true);
        let returned = start + 3 * PERIOD;

        assert_eq!(schedule.poll(start), Action::Advance);
        schedule.record_advance(0);
        assert!(!schedule.record_present(returned, true));
        schedule.set_available(returned, false);
        assert_eq!(
            schedule.poll(start + STALL_TIMEOUT),
            Action::WaitUntil(returned + STALL_TIMEOUT)
        );
        assert_eq!(schedule.poll(returned + STALL_TIMEOUT), Action::Stalled);
    }

    #[test]
    fn stall_takes_priority_over_a_frame_at_the_same_deadline() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 1, 1, true);
        let stall = start + STALL_TIMEOUT;

        assert_eq!(schedule.poll(start), Action::Advance);
        schedule.record_advance(0);
        assert!(!schedule.record_present(start, false));
        let before = stall.checked_sub(Duration::from_secs(1)).unwrap();
        assert_eq!(schedule.poll(before), Action::Retry(0));
        assert!(!schedule.record_present(before, false));
        assert_eq!(schedule.poll(before), Action::WaitUntil(stall));
        assert_eq!(schedule.poll(stall), Action::Stalled);
    }

    #[test]
    fn unavailable_surface_waits_for_stall_even_after_repeated_events() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 1, false);
        let stall = start + STALL_TIMEOUT;

        assert_eq!(schedule.poll(start), Action::WaitUntil(stall));
        schedule.set_available(start + PERIOD, false);
        assert_eq!(schedule.poll(start + PERIOD), Action::WaitUntil(stall));
        assert_eq!(schedule.poll(stall), Action::Stalled);
    }

    #[test]
    fn only_unavailable_to_available_resumes_immediately() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 1, true);
        let stall = start + STALL_TIMEOUT;

        assert_eq!(schedule.poll(start), Action::Advance);
        schedule.record_advance(0);
        assert!(!schedule.record_present(start, false));

        let resize = start + PERIOD / 4;
        schedule.set_available(resize, true);
        assert_eq!(schedule.poll(resize), Action::WaitUntil(start + PERIOD));

        let pause = start + PERIOD / 2;
        schedule.set_available(pause, false);
        assert_eq!(schedule.poll(pause), Action::WaitUntil(stall));

        let restore = start + 3 * PERIOD / 4;
        schedule.set_available(restore, true);
        assert_eq!(schedule.poll(restore), Action::Retry(0));
        assert!(!schedule.record_present(restore, false));
        schedule.set_available(restore, true);
        assert_eq!(schedule.poll(restore), Action::WaitUntil(restore + PERIOD));
        assert_eq!(schedule.poll(stall), Action::Stalled);
    }

    #[test]
    fn final_pending_tick_is_advanced_once_and_finished_only_after_a_successful_retry() {
        let start = Instant::now();
        let mut schedule = ScriptWindow::new(start, 4, 1, true);
        let (mut advances, mut finishes) = (0, 0);
        let mut attempts = Vec::new();
        let mut frame = |now, shown| {
            let tick = match schedule.poll(now) {
                Action::Advance => {
                    let tick = advances;
                    advances += 1;
                    schedule.record_advance(tick);
                    tick
                }
                Action::Retry(tick) => tick,
                action => panic!("expected a frame, got {action:?}"),
            };
            attempts.push(tick);
            if schedule.record_present(now, shown) {
                finishes += 1;
            }
            (advances, finishes)
        };

        assert_eq!(frame(start, true), (1, 0));
        assert_eq!(frame(start + PERIOD, false), (2, 0));
        assert_eq!(frame(start + 2 * PERIOD, true), (2, 1));
        assert_eq!(attempts, [0, 1, 1]);
    }
}
