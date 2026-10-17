pub(crate) mod decode;
pub(crate) mod device;
mod mixer;

use std::f32::consts::FRAC_PI_4;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use glam::Vec3;
use rtrb::{Producer, RingBuffer};

use crate::camera::Camera3D;

pub use mixer::MAX_RATE;
use mixer::{Command, Counters, LOOPS, Mixer};

/// Команд в очереди от игры к аудиопотоку.
const QUEUE: usize = 256;

/// Стерео-интерлив f32 в частоте `Assets`; пустой — заглушка ошибки загрузки.
#[derive(Clone, Debug)]
pub struct Sound {
    pub(crate) frames: Arc<[f32]>,
}

/// Громкости левого и правого канала эффекта.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stereo {
    /// Громкость левого канала.
    pub left: f32,
    /// Громкость правого канала.
    pub right: f32,
}

impl Stereo {
    /// Эффект в `source` для `listener`: панорама равной мощности по `yaw`, затухание `max(1 − d/range, 0)²`.
    pub fn at(listener: &Camera3D, source: Vec3, volume: f32, range: f32) -> Self {
        let to = source - listener.pos;
        let right = Vec3::new(listener.yaw.cos(), 0.0, -listener.yaw.sin());
        let pan = to.normalize_or_zero().dot(right);
        let fade = (1.0 - to.length() / range).max(0.0);
        let gain = volume * fade * fade;
        let angle = (pan + 1.0) * FRAC_PI_4;
        Self {
            left: gain * angle.cos(),
            right: gain * angle.sin(),
        }
    }

    pub(crate) fn both(volume: f32) -> Self {
        Self {
            left: volume,
            right: volume,
        }
    }
}

/// Длящийся голос: слот микшера и поколение его занятия.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopId {
    slot: usize,
    generation: u32,
}

/// Слот длящегося голоса глазами игры и его недоставленные команды.
#[derive(Default)]
struct LoopSlot {
    generation: u32,
    live: bool,
    /// Последние `(rate, volume)`, ещё не ушедшие в очередь.
    set: Option<(f32, f32)>,
    /// Поколение, чей `Stop` ещё не ушёл в очередь.
    stop: Option<u32>,
}

/// Запуск звуков из игры; `Default` — немой, команды никуда не идут.
#[derive(Default)]
pub struct Audio {
    queue: Option<Producer<Command>>,
    counters: Arc<Counters>,
    loops: [LoopSlot; LOOPS],
    /// С какого слота искать свободный: только что остановленный ещё затухает.
    next_loop: usize,
}

impl Audio {
    /// Игровая сторона и микшер аудиопотока над общей очередью.
    pub(crate) fn open() -> (Self, Mixer) {
        let (producer, consumer) = RingBuffer::new(QUEUE);
        let counters = Arc::new(Counters::default());
        let mixer = Mixer::new(consumer, counters.clone());
        let audio = Self {
            queue: Some(producer),
            counters,
            ..Self::default()
        };
        (audio, mixer)
    }

    /// Эффект поверх остальных; все голоса заняты — отбрасывается.
    pub fn play(&mut self, sound: &Sound, volume: f32) {
        self.effect(sound, Stereo::both(volume), false);
    }

    /// Эффект в пространстве: файл сводится в моно и звучит с громкостями `gains`.
    pub fn play_stereo(&mut self, sound: &Sound, gains: Stereo) {
        self.effect(sound, gains, true);
    }

    fn effect(&mut self, sound: &Sound, gains: Stereo, downmix: bool) {
        self.flush();
        let frames = sound.frames.clone();
        self.send(Command::Play {
            frames,
            gains,
            downmix,
        });
    }

    /// Музыка по кругу; играющую обрывает со щелчком — звать из `init`.
    pub fn play_music(&mut self, sound: &Sound, volume: f32) {
        self.flush();
        let frames = sound.frames.clone();
        self.send(Command::Music { frames, volume });
    }

    /// Голос по кругу до `stop_loop`; `None`, если все слоты заняты, очередь полна или звук немой.
    pub fn start_loop(&mut self, sound: &Sound, volume: f32) -> Option<LoopId> {
        self.flush();
        let queue = self.queue.as_mut()?;
        let slot = (0..LOOPS)
            .map(|k| (self.next_loop + k) % LOOPS)
            .find(|&slot| !self.loops[slot].live)?;
        let generation = self.loops[slot].generation.wrapping_add(1);
        let frames = sound.frames.clone();
        let start = Command::LoopStart {
            slot,
            generation,
            frames,
            volume,
        };
        if queue.push(start).is_err() {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let entry = &mut self.loops[slot];
        entry.generation = generation;
        entry.live = true;
        self.next_loop = slot + 1;
        Some(LoopId { slot, generation })
    }

    /// Новые скорость и громкость голоса; при полной очереди уходит последнее значение позже.
    pub fn set_loop(&mut self, id: LoopId, rate: f32, volume: f32) {
        if let Some(entry) = self.live_loop(id) {
            entry.set = Some((rate, volume));
        }
        self.flush();
    }

    /// Затухание голоса; при полной очереди `Stop` дойдёт позже, но не потеряется.
    pub fn stop_loop(&mut self, id: LoopId) {
        if let Some(entry) = self.live_loop(id) {
            entry.live = false;
            entry.set = None;
            entry.stop = Some(id.generation);
        }
        self.flush();
    }

    /// Досылает отложенные `Set` и `Stop`; app зовёт раз в кадр.
    pub(crate) fn flush(&mut self) {
        let Some(queue) = &mut self.queue else {
            return;
        };
        for (slot, entry) in self.loops.iter_mut().enumerate() {
            if let Some(generation) = entry.stop {
                if queue.push(Command::LoopStop { slot, generation }).is_err() {
                    return;
                }
                entry.stop = None;
            }
            if let Some((rate, volume)) = entry.set {
                let set = Command::LoopSet {
                    slot,
                    generation: entry.generation,
                    rate,
                    volume,
                };
                if queue.push(set).is_err() {
                    return;
                }
                entry.set = None;
            }
        }
    }

    fn live_loop(&mut self, id: LoopId) -> Option<&mut LoopSlot> {
        self.loops
            .get_mut(id.slot)
            .filter(|entry| entry.live && entry.generation == id.generation)
    }

    /// Счётчики аудиопотока; максимум колбэка сбрасывается. У немого — `None`.
    pub(crate) fn report(&self) -> Option<AudioReport> {
        self.queue.as_ref()?;
        let c = &self.counters;
        Some(AudioReport {
            callback_max: Duration::from_nanos(c.callback_max.swap(0, Ordering::Relaxed)),
            budget: Duration::from_nanos(c.budget.load(Ordering::Relaxed)),
            dropped: c.dropped.load(Ordering::Relaxed),
        })
    }

    fn send(&mut self, command: Command) {
        if let Some(queue) = &mut self.queue
            && queue.push(command).is_err()
        {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Диагностика аудиопотока для лога; `dropped` — с запуска.
#[derive(Debug, PartialEq)]
pub(crate) struct AudioReport {
    pub(crate) callback_max: Duration,
    pub(crate) budget: Duration,
    pub(crate) dropped: u64,
}

/// Сэмпл формата устройства.
pub(crate) trait DeviceSample: Copy {
    fn from_f32(s: f32) -> Self;
}

impl DeviceSample for f32 {
    fn from_f32(s: f32) -> Self {
        s
    }
}

impl DeviceSample for i16 {
    fn from_f32(s: f32) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "микшер клипует в [-1, 1], значит s * 32767 — в диапазоне i16"
        )]
        let s = (s * 32767.0).round() as i16;
        s
    }
}

/// Буфер устройства из `channels` каналов, кусками по `scratch`: моно — среднее L и R, сверх двух — тишина.
pub(crate) fn fill<S: DeviceSample>(
    mixer: &mut Mixer,
    scratch: &mut [f32],
    out: &mut [S],
    channels: usize,
) {
    for chunk in out.chunks_mut(scratch.len() / 2 * channels) {
        let mix = &mut scratch[..chunk.len() / channels * 2];
        mixer.render(mix);
        for (device, &[l, r]) in chunk.chunks_exact_mut(channels).zip(mix.as_chunks().0) {
            if channels == 1 {
                device[0] = S::from_f32(f32::midpoint(l, r));
            } else {
                device[0] = S::from_f32(l);
                device[1] = S::from_f32(r);
                device[2..].fill(S::from_f32(0.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, TAU};
    use std::iter;

    use rtrb::Consumer;

    use super::*;

    fn sound(frames: &[[f32; 2]]) -> Sound {
        Sound {
            frames: frames.iter().flatten().copied().collect(),
        }
    }

    /// Различимые фреймы без огибающей на кусках: фрейм `i` — `(i + 1) / 1000` и его минус.
    fn ramp(frames: usize) -> Sound {
        let frames: Vec<[f32; 2]> = (0..frames)
            .map(|i| {
                let s = (i + 1) as f32 / 1000.0;
                [s, -s]
            })
            .collect();
        sound(&frames)
    }

    /// `Audio` над очередью `cap` команд, которую тест читает сам.
    fn queued(cap: usize) -> (Audio, Consumer<Command>) {
        let (tx, rx) = RingBuffer::new(cap);
        let audio = Audio {
            queue: Some(tx),
            ..Audio::default()
        };
        (audio, rx)
    }

    fn drain(rx: &mut Consumer<Command>) -> Vec<String> {
        iter::from_fn(|| rx.pop().ok())
            .map(|command| match command {
                Command::Play { .. } => "play".to_owned(),
                Command::Music { .. } => "music".to_owned(),
                Command::LoopStart {
                    slot, generation, ..
                } => format!("start {slot}/{generation}"),
                Command::LoopSet {
                    slot,
                    generation,
                    rate,
                    volume,
                } => format!("set {slot}/{generation} {rate} {volume}"),
                Command::LoopStop { slot, generation } => format!("stop {slot}/{generation}"),
            })
            .collect()
    }

    fn id(slot: usize, generation: u32) -> LoopId {
        LoopId { slot, generation }
    }

    /// Музыка из одного фрейма после fade-in играет ровно его × 0,5.
    fn steady(l: f32, r: f32) -> Mixer {
        let (mut audio, mut mixer) = Audio::open();
        audio.play_music(&sound(&[[l, r]]), 1.0);
        mixer.render(&mut [0.0; 128]);
        mixer
    }

    #[test]
    fn muted_audio_ignores_commands() {
        let mut audio = Audio::default();
        audio.play(&ramp(10), 1.0);
        audio.play_music(&ramp(10), 1.0);
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn muted_audio_has_no_loops() {
        let mut audio = Audio::default();
        assert_eq!(audio.start_loop(&ramp(10), 1.0), None);
        audio.set_loop(id(0, 1), 1.0, 1.0);
        audio.stop_loop(id(0, 1));
        audio.flush();
    }

    #[test]
    fn start_loop_on_full_queue_returns_none_and_keeps_slot() {
        let (mut audio, mut rx) = queued(1);
        let s = ramp(10);
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(0, 1)));
        assert_eq!(audio.start_loop(&s, 1.0), None);
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 1);
        assert_eq!(drain(&mut rx), ["start 0/1"]);
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(1, 1)));
    }

    #[test]
    fn start_loop_without_free_slot_returns_none() {
        let (mut audio, mut rx) = queued(QUEUE);
        let s = ramp(10);
        for slot in 0..LOOPS {
            assert_eq!(audio.start_loop(&s, 1.0), Some(id(slot, 1)));
        }
        assert_eq!(audio.start_loop(&s, 1.0), None);
        assert_eq!(drain(&mut rx).len(), LOOPS);
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn loops_take_slots_round_robin_and_bump_generation() {
        let (mut audio, mut rx) = queued(QUEUE);
        let s = ramp(10);
        let first = audio.start_loop(&s, 1.0).unwrap();
        audio.stop_loop(first);
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(1, 1)));
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(2, 1)));
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(3, 1)));
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(0, 2)));
        audio.stop_loop(id(3, 1));
        audio.stop_loop(id(2, 1));
        assert_eq!(audio.start_loop(&s, 1.0), Some(id(2, 2)));
        assert_eq!(
            drain(&mut rx),
            [
                "start 0/1",
                "stop 0/1",
                "start 1/1",
                "start 2/1",
                "start 3/1",
                "start 0/2",
                "stop 3/1",
                "stop 2/1",
                "start 2/2"
            ]
        );
    }

    #[test]
    fn stop_waits_out_full_queue_until_flush() {
        let (mut audio, mut rx) = queued(1);
        let voice = audio.start_loop(&ramp(10), 1.0).unwrap();
        audio.stop_loop(voice);
        assert_eq!(drain(&mut rx), ["start 0/1"]);
        audio.flush();
        assert_eq!(drain(&mut rx), ["stop 0/1"]);
        audio.flush();
        assert_eq!(drain(&mut rx), Vec::<String>::new());
    }

    #[test]
    fn stop_waits_out_full_queue_until_next_call() {
        let (mut audio, mut rx) = queued(2);
        let s = ramp(10);
        let voice = audio.start_loop(&s, 1.0).unwrap();
        audio.play(&s, 1.0);
        audio.stop_loop(voice);
        assert_eq!(drain(&mut rx), ["start 0/1", "play"]);
        audio.play(&s, 1.0);
        assert_eq!(drain(&mut rx), ["stop 0/1", "play"]);
    }

    #[test]
    fn set_loop_coalesces_to_last_while_queue_is_full() {
        let (mut audio, mut rx) = queued(1);
        let voice = audio.start_loop(&ramp(10), 1.0).unwrap();
        audio.set_loop(voice, 1.5, 0.5);
        audio.set_loop(voice, 2.5, 0.25);
        assert_eq!(drain(&mut rx), ["start 0/1"]);
        audio.flush();
        assert_eq!(drain(&mut rx), ["set 0/1 2.5 0.25"]);
        audio.flush();
        assert_eq!(drain(&mut rx), Vec::<String>::new());
    }

    #[test]
    fn stop_drops_pending_set_and_stale_ids_are_ignored() {
        let (mut audio, mut rx) = queued(1);
        let s = ramp(10);
        let old = audio.start_loop(&s, 1.0).unwrap();
        audio.set_loop(old, 2.0, 1.0);
        audio.stop_loop(old);
        drain(&mut rx);
        audio.flush();
        assert_eq!(drain(&mut rx), ["stop 0/1"]);

        audio.set_loop(old, 3.0, 1.0);
        audio.stop_loop(old);
        audio.set_loop(id(1, 1), 3.0, 1.0);
        audio.set_loop(id(LOOPS, 0), 3.0, 1.0);
        audio.flush();
        assert_eq!(drain(&mut rx), Vec::<String>::new());
    }

    #[test]
    fn stopped_loop_fades_to_silence_in_mixer() {
        let (mut audio, mut mixer) = Audio::open();
        let voice = audio.start_loop(&sound(&[[1.0, 1.0]]), 1.0).unwrap();
        audio.set_loop(voice, 1.0, 1.0);
        let mut out = [0.0; 256];
        mixer.render(&mut out);
        assert_eq!(out[254], 0.5);
        audio.stop_loop(voice);
        mixer.render(&mut out);
        assert_eq!(out[254..], [0.0; 2]);
    }

    #[test]
    fn muted_audio_has_no_report() {
        assert_eq!(Audio::default().report(), None);
    }

    #[test]
    fn report_takes_callback_max_and_keeps_budget_and_dropped() {
        let (audio, _mixer) = Audio::open();
        let c = &audio.counters;
        c.record_callback(Duration::from_millis(3), 480, 48_000);
        c.record_callback(Duration::from_millis(1), 960, 48_000);
        c.dropped.fetch_add(2, Ordering::Relaxed);
        assert_eq!(
            audio.report(),
            Some(AudioReport {
                callback_max: Duration::from_millis(3),
                budget: Duration::from_millis(20),
                dropped: 2,
            })
        );
        let next = audio.report().unwrap();
        assert_eq!(next.callback_max, Duration::ZERO);
        assert_eq!(next.budget, Duration::from_millis(20));
        assert_eq!(next.dropped, 2);
    }

    #[test]
    fn play_and_music_reach_mixer() {
        let (mut audio, mut mixer) = Audio::open();
        audio.play(&sound(&[[0.0, 0.0], [1.0, 1.0], [0.0, 0.0]]), 64.0);
        let mut out = [0.0; 6];
        mixer.render(&mut out);
        assert_eq!(out[2], 0.5);

        audio.play_music(&sound(&[[1.0, 1.0]]), 64.0);
        let mut out = [0.0; 4];
        mixer.render(&mut out);
        assert_eq!(out, [0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn play_keeps_file_channels_and_play_stereo_downmixes() {
        let (mut audio, mut mixer) = Audio::open();
        let s = sound(&[[0.0, 0.0], [1.0, -0.5], [0.0, 0.0]]);
        audio.play(&s, 64.0);
        let mut out = [0.0; 6];
        mixer.render(&mut out);
        assert_eq!(out[2..4], [0.5, -0.25]);

        let gains = Stereo {
            left: 64.0,
            right: 32.0,
        };
        audio.play_stereo(&s, gains);
        mixer.render(&mut out);
        assert_eq!(out[2..4], [0.125, 0.0625]);
    }

    fn eye(yaw: f32, pitch: f32) -> Camera3D {
        Camera3D {
            pos: Vec3::new(1.0, 2.0, 3.0),
            yaw,
            pitch,
            ..Camera3D::default()
        }
    }

    /// Источник на расстоянии 5 под углом `angle` от взгляда, против часовой сверху.
    fn around(listener: &Camera3D, angle: f32) -> Stereo {
        let yaw = listener.yaw + angle;
        let dir = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
        Stereo::at(listener, listener.pos + dir * 5.0, 2.0, 10.0)
    }

    #[test]
    fn source_on_the_right_is_louder_in_right_channel() {
        for yaw in [0.0, 1.0, -2.5] {
            let right = around(&eye(yaw, 0.0), -0.5);
            assert!(right.right > right.left, "{yaw}: {right:?}");
            let left = around(&eye(yaw, 0.0), 0.5);
            assert!(left.left > left.right, "{yaw}: {left:?}");
        }
        let side = around(&eye(0.3, 0.0), -FRAC_PI_2);
        assert!(side.left.abs() < 1e-6, "{side:?}");
        assert!((side.right - 0.5).abs() < 1e-6, "{side:?}");
    }

    #[test]
    fn source_ahead_splits_equal_power_with_light_falloff() {
        let ahead = around(&eye(0.7, 0.0), 0.0);
        let expected = 2.0 * 0.25 * FRAC_1_SQRT_2;
        assert!((ahead.left - expected).abs() < 1e-6, "{ahead:?}");
        assert!((ahead.right - expected).abs() < 1e-6, "{ahead:?}");
    }

    #[test]
    fn source_in_the_eye_is_centered_at_full_volume() {
        let listener = eye(0.7, 0.0);
        let at = Stereo::at(&listener, listener.pos, 2.0, 10.0);
        assert!((at.left - 2.0 * FRAC_1_SQRT_2).abs() < 1e-6, "{at:?}");
        assert!((at.right - 2.0 * FRAC_1_SQRT_2).abs() < 1e-6, "{at:?}");
    }

    #[test]
    fn source_at_or_beyond_range_is_silent() {
        let listener = eye(0.0, 0.0);
        for z in [-10.0, -11.0, -100.0] {
            let at = Stereo::at(&listener, listener.pos + Vec3::new(0.0, 0.0, z), 2.0, 10.0);
            assert_eq!(at, Stereo::both(0.0), "{z}");
        }
    }

    #[test]
    fn power_is_constant_around_listener() {
        let listener = eye(0.4, 0.0);
        for k in 0..16 {
            let at = around(&listener, k as f32 * TAU / 16.0);
            let power = at.left.powi(2) + at.right.powi(2);
            assert!((power - 0.25).abs() < 1e-6, "{k}: {at:?}");
        }
    }

    #[test]
    fn pitch_does_not_change_channels() {
        let source = Vec3::new(4.0, 0.0, -1.0);
        let level = Stereo::at(&eye(0.4, 0.0), source, 2.0, 10.0);
        for pitch in [1.2, -1.2] {
            assert_eq!(Stereo::at(&eye(0.4, pitch), source, 2.0, 10.0), level);
        }
    }

    #[test]
    fn full_queue_drops_play_with_counter() {
        let (mut audio, _mixer) = Audio::open();
        let s = ramp(10);
        for _ in 0..QUEUE {
            audio.play(&s, 1.0);
        }
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 0);
        audio.play(&s, 1.0);
        audio.play_music(&s, 1.0);
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn fill_in_chunks_neither_skips_nor_repeats_frames() {
        let (mut audio, mut whole) = Audio::open();
        audio.play(&ramp(1000), 1.0);
        let mut expected = vec![0.0; 500];
        whole.render(&mut expected);

        let (mut audio, mut mixer) = Audio::open();
        audio.play(&ramp(1000), 1.0);
        let mut scratch = [0.0; 200];
        let mut out = vec![9.0_f32; 500];
        fill(&mut mixer, &mut scratch, &mut out, 2);
        assert_eq!(out, expected);
    }

    #[test]
    fn callback_neither_allocates_nor_frees() {
        let (mut audio, mut mixer) = Audio::open();
        let short = ramp(100);
        let long = ramp(4096);
        let music = ramp(10);
        let next_music = ramp(20);
        let motor = ramp(30);
        let next_motor = ramp(40);
        let mut scratch = [0.0; 256];
        let mut out = [0.0_f32; 1024];
        let info = allocation_counter::measure(|| {
            audio.play(&short, 1.0);
            audio.play_stereo(&short, Stereo::both(1.0));
            audio.play_music(&music, 1.0);
            audio.play_music(&next_music, 1.0);
            let voices: [LoopId; LOOPS] =
                std::array::from_fn(|_| audio.start_loop(&motor, 1.0).unwrap());
            audio.set_loop(voices[0], 1.7, 0.5);
            fill(&mut mixer, &mut scratch, &mut out, 2);
            for _ in 0..33 {
                audio.play(&long, 1.0);
            }
            audio.stop_loop(voices[0]);
            audio.stop_loop(voices[1]);
            audio.start_loop(&next_motor, 1.0);
            fill(&mut mixer, &mut scratch, &mut out, 2);
            audio
                .counters
                .record_callback(Duration::from_millis(1), 512, 48_000);
        });
        assert_eq!(audio.counters.dropped.load(Ordering::Relaxed), 1);
        assert_eq!(info, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn fill_converts_to_i16() {
        let mut mixer = steady(1.0, -0.5);
        let mut out = [7_i16; 4];
        fill(&mut mixer, &mut [0.0; 8], &mut out, 2);
        assert_eq!(out, [16384, -8192, 16384, -8192]);
    }

    #[test]
    fn fill_mono_averages_channels() {
        let mut mixer = steady(1.0, 0.5);
        let mut out = [9.0_f32; 3];
        fill(&mut mixer, &mut [0.0; 4], &mut out, 1);
        assert_eq!(out, [0.375; 3]);
    }

    #[test]
    fn fill_extra_channels_are_silent() {
        let mut mixer = steady(1.0, -1.0);
        let mut out = [9_i16; 12];
        fill(&mut mixer, &mut [0.0; 4], &mut out, 6);
        assert_eq!(out, [16384, -16384, 0, 0, 0, 0, 16384, -16384, 0, 0, 0, 0]);
    }
}
