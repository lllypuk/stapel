//! Звук мотора: синтез цикла, виртуальная коробка передач и команды длящемуся голосу.

use std::f64::consts::TAU;

use stapel::audio::MAX_RATE;

use crate::Mode;

/// Обороты, на которых цикл играет с `rate = 1`.
const RPM_SAMPLE: f32 = 2000.0;
/// Вес нечётных гармоник оборота: вспышка четырёх цилиндров — дважды за оборот.
const ODD: f64 = 0.3;
const PEAK: f32 = 0.9;
const IDLE: f32 = 900.0;
const RED: f32 = 7000.0;
/// До этих оборотов сцепление даёт раскрутить мотор на месте.
const LAUNCH: f32 = 3500.0;
const UPSHIFT: f32 = 6300.0;
/// Вниз — когда на передаче ниже обороты не упадут под этот порог.
const DOWNSHIFT: f32 = 3800.0;
/// Доля `max_speed`, на которой передача выходит на `RED`.
const GEARS: [f32; 5] = [0.25, 0.45, 0.65, 0.83, 1.0];
/// Доля пути к целевым оборотам за тик.
const LAG: f32 = 0.3;
const VOLUME: f32 = 0.5;
/// Громкость без газа — доля `VOLUME`.
const IDLE_VOLUME: f32 = 0.4;

/// Один оборот коленвала на `RPM_SAMPLE`, стерео-интерлив; гармоники выше `sample_rate / 2 / MAX_RATE` завернулись бы на `MAX_RATE`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "оборот — тысяча с небольшим отсчётов, отсчёты нормированы к PEAK"
)]
pub(crate) fn synth(sample_rate: u32) -> Vec<f32> {
    let period = (f64::from(sample_rate) * 60.0 / f64::from(RPM_SAMPLE)).round() as u32;
    let limit = f64::from(period) / 2.0 / f64::from(MAX_RATE);
    let harmonics: Vec<u32> = (1..period).take_while(|&k| f64::from(k) < limit).collect();
    let mono: Vec<f64> = (0..period)
        .map(|j| {
            let wave = |&k: &u32| {
                let weight = if k % 2 == 0 { 1.0 } else { ODD };
                let phase = TAU * f64::from(k) * f64::from(j) / f64::from(period);
                weight / f64::from(k) * phase.sin()
            };
            harmonics.iter().map(wave).sum()
        })
        .collect();
    let scale = f64::from(PEAK) / mono.iter().fold(0.0, |m: f64, s| m.max(s.abs()));
    mono.iter().flat_map(|&s| [(s * scale) as f32; 2]).collect()
}

/// Обороты мотора по скорости, передаче и газу.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rpm {
    gear: usize,
    revs: f32,
    throttle: f32,
}

impl Default for Rpm {
    fn default() -> Self {
        Self {
            gear: 0,
            revs: IDLE,
            throttle: 0.0,
        }
    }
}

impl Rpm {
    /// Тик: `speed` — доля `max_speed`; в воздухе колёса мотор не держат.
    pub(crate) fn tick(&mut self, speed: f32, throttle: f32, grounded: bool) {
        let wheel = |gear: usize| RED * speed / GEARS[gear];
        let target = if grounded {
            if self.gear + 1 < GEARS.len() && wheel(self.gear) > UPSHIFT {
                self.gear += 1;
            } else if self.gear > 0 && wheel(self.gear - 1) < DOWNSHIFT {
                self.gear -= 1;
            }
            wheel(self.gear).max(IDLE + throttle * (LAUNCH - IDLE))
        } else {
            IDLE + throttle * (RED - IDLE)
        };
        self.revs += (target - self.revs) * LAG;
        self.throttle = throttle;
    }

    pub(crate) fn rate(&self) -> f32 {
        (self.revs / RPM_SAMPLE).clamp(0.0, MAX_RATE)
    }

    pub(crate) fn volume(&self) -> f32 {
        VOLUME * (IDLE_VOLUME + (1.0 - IDLE_VOLUME) * self.throttle)
    }
}

/// Что сделать с голосом мотора в этом тике.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SoundCmd {
    Start,
    /// `rate`, громкость.
    Set(f32, f32),
    Stop,
    Nothing,
}

/// Мотор звучит только в заезде; `playing` — голос получен от `start_loop`.
pub(crate) fn command(mode: Mode, rpm: &Rpm, playing: bool) -> SoundCmd {
    match (mode, playing) {
        (Mode::Race, false) => SoundCmd::Start,
        (Mode::Race, true) => SoundCmd::Set(rpm.rate(), rpm.volume()),
        (_, true) => SoundCmd::Stop,
        (_, false) => SoundCmd::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Амплитуды бинов ДПФ моно-канала, `0..=n/2`.
    fn spectrum(frames: &[f32]) -> Vec<f64> {
        let mono: Vec<f64> = frames.iter().step_by(2).map(|&s| f64::from(s)).collect();
        let n = mono.len();
        (0..=n / 2)
            .map(|bin| {
                let (re, im) = mono
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(re, im), (j, s)| {
                        let phase = TAU * ((bin * j) % n) as f64 / n as f64;
                        (re + s * phase.cos(), im - s * phase.sin())
                    });
                re.hypot(im) * 2.0 / n as f64
            })
            .collect()
    }

    /// Обороты после `ticks` тиков с одними входами.
    fn settle(rpm: &mut Rpm, speed: f32, throttle: f32, grounded: bool, ticks: u32) -> f32 {
        for _ in 0..ticks {
            rpm.tick(speed, throttle, grounded);
        }
        rpm.revs
    }

    #[test]
    fn synth_is_one_revolution_of_stereo_peaking_at_peak() {
        let frames = synth(48_000);
        assert_eq!(frames.len(), 2 * 1440);
        assert_eq!(synth(44_100).len(), 2 * 1323);
        assert!(frames.chunks(2).all(|f| f[0].to_bits() == f[1].to_bits()));
        let peak = frames.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak - PEAK).abs() < 1e-6, "{peak}");
    }

    #[test]
    fn synth_loops_seamlessly() {
        let frames = synth(48_000);
        let mono: Vec<f32> = frames.iter().step_by(2).copied().collect();
        let step = mono
            .windows(2)
            .fold(0.0_f32, |m, w| m.max((w[1] - w[0]).abs()));
        let seam = (mono[0] - mono[mono.len() - 1]).abs();
        assert!(seam <= step, "{seam} > {step}");
    }

    #[test]
    fn synth_has_no_harmonics_that_alias_at_max_rate() {
        let mags = spectrum(&synth(48_000));
        // 1440 отсчётов на оборот: бин 180 при MAX_RATE = 4 — ровно Найквист.
        let cutoff = 180;
        assert!(mags[cutoff - 1] > 1e-3, "{}", mags[cutoff - 1]);
        let above = mags[cutoff..].iter().fold(0.0, |m: f64, &a| m.max(a));
        assert!(above < 1e-5, "{above}");
    }

    #[test]
    fn firing_harmonic_dominates_and_harmonics_fall_with_order() {
        let mags = spectrum(&synth(48_000));
        let loudest = (0..mags.len()).max_by(|&a, &b| mags[a].total_cmp(&mags[b]));
        assert_eq!(loudest, Some(2));
        assert!(mags[0] < 1e-9, "{}", mags[0]);
        for k in (2..170).step_by(2) {
            assert!(mags[k] > mags[k + 2], "{k}");
            assert!(mags[k - 1] < mags[k], "{k}");
            assert!(mags[k + 1] < mags[k], "{k}");
        }
    }

    #[test]
    fn rpm_rises_with_speed_within_a_gear() {
        let mut rpm = Rpm::default();
        let slow = settle(&mut rpm, 0.1, 0.0, true, 60);
        let fast = settle(&mut rpm, 0.2, 0.0, true, 60);
        assert_eq!(rpm.gear, 0);
        assert!((slow - RED * 0.4).abs() < 1.0, "{slow}");
        assert!((fast - RED * 0.8).abs() < 1.0, "{fast}");
    }

    #[test]
    fn upshift_drops_rpm_and_downshift_waits_for_low_revs() {
        let mut rpm = Rpm::default();
        let before = settle(&mut rpm, 0.22, 1.0, true, 60);
        assert_eq!(rpm.gear, 0);
        let after = settle(&mut rpm, 0.23, 1.0, true, 1);
        assert_eq!(rpm.gear, 1);
        assert!(after < before, "{after} >= {before}");
        settle(&mut rpm, 0.14, 0.0, true, 1);
        assert_eq!(rpm.gear, 1);
        settle(&mut rpm, 0.13, 0.0, true, 1);
        assert_eq!(rpm.gear, 0);
        settle(&mut rpm, 2.0, 1.0, true, 60);
        assert_eq!(rpm.gear, GEARS.len() - 1);
    }

    #[test]
    fn standstill_and_air_follow_throttle() {
        let mut rpm = Rpm::default();
        let first = settle(&mut rpm, 0.0, 1.0, true, 1);
        assert!(
            (first - (IDLE + (LAUNCH - IDLE) * LAG)).abs() < 1e-3,
            "{first}"
        );
        assert!((settle(&mut rpm, 0.0, 0.0, true, 60) - IDLE).abs() < 1.0);
        assert!((settle(&mut rpm, 0.0, 1.0, true, 60) - LAUNCH).abs() < 1.0);
        let mut rpm = Rpm::default();
        settle(&mut rpm, 0.9, 1.0, true, 60);
        let gear = rpm.gear;
        assert!((settle(&mut rpm, 0.9, 0.0, false, 60) - IDLE).abs() < 1.0);
        assert!((settle(&mut rpm, 0.9, 1.0, false, 60) - RED).abs() < 1.0);
        assert_eq!(rpm.gear, gear);
    }

    #[test]
    fn rate_follows_rpm_within_max_rate_and_volume_follows_throttle() {
        let mut rpm = Rpm::default();
        settle(&mut rpm, 0.0, 0.0, true, 60);
        assert!(
            (rpm.rate() - IDLE / RPM_SAMPLE).abs() < 1e-3,
            "{}",
            rpm.rate()
        );
        let idle = rpm.volume();
        settle(&mut rpm, 3.0, 1.0, true, 60);
        assert_eq!(rpm.rate(), MAX_RATE);
        assert!((idle - VOLUME * IDLE_VOLUME).abs() < 1e-6, "{idle}");
        assert!((rpm.volume() - VOLUME).abs() < 1e-6, "{}", rpm.volume());
    }

    #[test]
    fn command_starts_sets_and_stops_by_mode() {
        let mut rpm = Rpm::default();
        settle(&mut rpm, 0.1, 1.0, true, 10);
        assert_eq!(command(Mode::Race, &rpm, false), SoundCmd::Start);
        assert_eq!(
            command(Mode::Race, &rpm, true),
            SoundCmd::Set(rpm.rate(), rpm.volume())
        );
        assert_eq!(command(Mode::Garage, &rpm, true), SoundCmd::Stop);
        assert_eq!(command(Mode::Garage, &rpm, false), SoundCmd::Nothing);
    }
}
