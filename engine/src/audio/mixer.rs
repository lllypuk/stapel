use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rtrb::Consumer;

/// Голосов эффектов; музыка — отдельный слот сверх них.
const VOICES: usize = 32;
/// Длящихся голосов, отдельно от эффектов.
pub(crate) const LOOPS: usize = 4;
/// Предел скорости проигрывания длящегося голоса.
pub const MAX_RATE: f32 = 4.0;
/// Длина fade-in и fade-out в аудиофреймах.
const FADE: usize = 64;
/// Команд из очереди за один `render`.
const COMMANDS_PER_RENDER: usize = 64;
const MASTER: f32 = 0.5;

/// Команда игры аудиопотоку; `frames` — стерео-интерлив.
pub(crate) enum Command {
    /// Эффект в свободный голос.
    Play { frames: Arc<[f32]>, volume: f32 },
    /// Музыка по кругу вместо играющей.
    Music { frames: Arc<[f32]>, volume: f32 },
    /// Длящийся голос в `slot` вместо прежнего.
    LoopStart {
        slot: usize,
        generation: u32,
        frames: Arc<[f32]>,
        volume: f32,
    },
    /// Новые скорость и громкость голоса `slot` поколения `generation`.
    LoopSet {
        slot: usize,
        generation: u32,
        rate: f32,
        volume: f32,
    },
    /// Затухание и освобождение голоса `slot` поколения `generation`.
    LoopStop { slot: usize, generation: u32 },
}

/// Счётчики, общие для игры и аудиопотока.
#[derive(Default)]
pub(crate) struct Counters {
    /// Команды, отброшенные из-за полной очереди или занятых голосов.
    pub(crate) dropped: AtomicU64,
    /// Самый долгий колбэк устройства с прошлого отчёта, нс.
    pub(crate) callback_max: AtomicU64,
    /// Длительность звука в последнем буфере устройства, нс.
    pub(crate) budget: AtomicU64,
}

impl Counters {
    /// Колбэк устройства занял `took` на буфер из `frames` фреймов в частоте `rate`.
    pub(crate) fn record_callback(&self, took: Duration, frames: usize, rate: u32) {
        let budget = Duration::from_secs_f64(frames as f64 / f64::from(rate));
        self.callback_max.fetch_max(nanos(took), Ordering::Relaxed);
        self.budget.store(nanos(budget), Ordering::Relaxed);
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

struct Voice {
    frames: Arc<[f32]>,
    pos: usize,
    /// Фреймов с первого старта, до `FADE`: на стыке цикла не сбрасывается.
    age: usize,
    volume: f32,
    looping: bool,
}

impl Voice {
    fn new(frames: Arc<[f32]>, volume: f32, looping: bool) -> Option<Self> {
        (!frames.is_empty()).then_some(Self {
            frames,
            pos: 0,
            age: 0,
            volume,
            looping,
        })
    }

    /// Подмешивает себя в `out`; `true` — звук кончился.
    fn mix(&mut self, out: &mut [f32]) -> bool {
        let len = self.frames.len() / 2;
        for frame in out.as_chunks_mut::<2>().0 {
            let tail = if self.looping {
                FADE
            } else {
                len - 1 - self.pos
            };
            let gain = self.volume * self.age.min(tail) as f32 / FADE as f32;
            frame[0] += self.frames[2 * self.pos] * gain;
            frame[1] += self.frames[2 * self.pos + 1] * gain;
            self.age = (self.age + 1).min(FADE);
            self.pos += 1;
            if self.pos == len {
                if !self.looping {
                    return true;
                }
                self.pos = 0;
            }
        }
        false
    }
}

/// Голос по кругу с живыми скоростью и громкостью; играет до `LoopStop`.
struct LoopVoice {
    frames: Arc<[f32]>,
    generation: u32,
    /// Дробный фрейм в `[0, len)`.
    phase: f64,
    rate: f32,
    volume: f32,
    target_rate: f32,
    target_volume: f32,
    /// Огибающая в фреймах, `0..=FADE`.
    fade: usize,
    stopping: bool,
}

impl LoopVoice {
    fn new(frames: Arc<[f32]>, generation: u32, volume: f32) -> Option<Self> {
        (!frames.is_empty()).then_some(Self {
            frames,
            generation,
            phase: 0.0,
            rate: 1.0,
            volume,
            target_rate: 1.0,
            target_volume: volume,
            fade: 0,
            stopping: false,
        })
    }

    /// Подмешивает себя в `out`, ведя скорость и громкость к целевым за буфер; `true` — затих после стопа.
    fn mix(&mut self, out: &mut [f32]) -> bool {
        let len = self.frames.len() / 2;
        let frames = out.as_chunks_mut::<2>().0;
        let count = frames.len() as f32;
        let (rate, volume) = (self.rate, self.volume);
        for (k, frame) in frames.iter_mut().enumerate() {
            let along = (k + 1) as f32 / count;
            self.rate = rate + (self.target_rate - rate) * along;
            self.volume = volume + (self.target_volume - volume) * along;
            if self.stopping {
                self.fade = self.fade.saturating_sub(1);
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "phase в [0, len)"
            )]
            let (i, frac) = (self.phase as usize, self.phase.fract() as f32);
            let j = if i + 1 == len { 0 } else { i + 1 };
            let gain = self.volume * self.fade as f32 / FADE as f32;
            let here = &self.frames[2 * i..2 * i + 2];
            let next = &self.frames[2 * j..2 * j + 2];
            for ((out, here), next) in frame.iter_mut().zip(here).zip(next) {
                *out += (here + (next - here) * frac) * gain;
            }
            if self.stopping {
                if self.fade == 0 {
                    return true;
                }
            } else {
                self.fade = (self.fade + 1).min(FADE);
            }
            self.phase = (self.phase + f64::from(self.rate)) % len as f64;
        }
        false
    }

    /// Голос `slot`, если он поколения `generation`.
    fn find(loops: &mut [Option<Self>], slot: usize, generation: u32) -> Option<&mut Self> {
        loops
            .get_mut(slot)?
            .as_mut()
            .filter(|voice| voice.generation == generation)
    }
}

/// Микс голосов аудиопотока; команды берёт из очереди `Audio`.
pub(crate) struct Mixer {
    queue: Consumer<Command>,
    voices: [Option<Voice>; VOICES],
    music: Option<Voice>,
    loops: [Option<LoopVoice>; LOOPS],
    counters: Arc<Counters>,
}

impl Mixer {
    pub(crate) fn new(queue: Consumer<Command>, counters: Arc<Counters>) -> Self {
        Self {
            queue,
            voices: [const { None }; VOICES],
            music: None,
            loops: [const { None }; LOOPS],
            counters,
        }
    }

    /// Стерео-интерлив в `out`: сумма голосов × `MASTER`, клип в `[-1, 1]`. Без аллокаций.
    pub(crate) fn render(&mut self, out: &mut [f32]) {
        for _ in 0..COMMANDS_PER_RENDER {
            let Ok(command) = self.queue.pop() else {
                break;
            };
            self.start(command);
        }
        out.fill(0.0);
        for slot in self.voices.iter_mut().chain(iter::once(&mut self.music)) {
            if slot.as_mut().is_some_and(|voice| voice.mix(out)) {
                *slot = None;
            }
        }
        for slot in &mut self.loops {
            if slot.as_mut().is_some_and(|voice| voice.mix(out)) {
                *slot = None;
            }
        }
        for s in out {
            *s = (*s * MASTER).clamp(-1.0, 1.0);
        }
    }

    fn start(&mut self, command: Command) {
        match command {
            Command::Play { frames, volume } => {
                let Some(voice) = Voice::new(frames, volume, false) else {
                    return;
                };
                // Без кражи голоса: обрыв играющего щёлкает.
                match self.voices.iter_mut().find(|slot| slot.is_none()) {
                    Some(slot) => *slot = Some(voice),
                    None => {
                        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Command::Music { frames, volume } => {
                if let Some(voice) = Voice::new(frames, volume, true) {
                    self.music = Some(voice);
                }
            }
            Command::LoopStart {
                slot,
                generation,
                frames,
                volume,
            } => {
                if let (Some(slot), Some(voice)) = (
                    self.loops.get_mut(slot),
                    LoopVoice::new(frames, generation, volume),
                ) {
                    *slot = Some(voice);
                }
            }
            Command::LoopSet {
                slot,
                generation,
                rate,
                volume,
            } => {
                if let Some(voice) = LoopVoice::find(&mut self.loops, slot, generation) {
                    // NaN в фазе — вечный NaN на выходе.
                    voice.target_rate = if rate.is_nan() {
                        0.0
                    } else {
                        rate.clamp(0.0, MAX_RATE)
                    };
                    voice.target_volume = volume;
                }
            }
            Command::LoopStop { slot, generation } => {
                if let Some(voice) = LoopVoice::find(&mut self.loops, slot, generation) {
                    voice.stopping = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::{Producer, RingBuffer};

    fn setup() -> (Producer<Command>, Mixer) {
        let (tx, rx) = RingBuffer::new(256);
        (tx, Mixer::new(rx, Arc::default()))
    }

    fn constant(frames: usize, l: f32, r: f32) -> Arc<[f32]> {
        iter::repeat_n([l, r], frames).flatten().collect()
    }

    /// Различимые значения: фрейм `i` — `(i + 1) / 1000` и его минус.
    fn ramp(frames: usize) -> Arc<[f32]> {
        (0..frames)
            .flat_map(|i| {
                let s = (i + 1) as f32 / 1000.0;
                [s, -s]
            })
            .collect()
    }

    fn play(tx: &mut Producer<Command>, frames: &Arc<[f32]>, volume: f32) {
        let frames = frames.clone();
        assert!(tx.push(Command::Play { frames, volume }).is_ok());
    }

    fn music(tx: &mut Producer<Command>, frames: &Arc<[f32]>, volume: f32) {
        let frames = frames.clone();
        assert!(tx.push(Command::Music { frames, volume }).is_ok());
    }

    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![9.0; frames * 2];
        mixer.render(&mut out);
        out
    }

    /// Огибающая эффекта длиной `len` во фрейме `i`.
    fn envelope(i: usize, len: usize) -> f32 {
        i.min(len - 1 - i).min(FADE) as f32 / FADE as f32
    }

    #[test]
    fn voice_is_scaled_by_volume_and_master() {
        let (mut tx, mut mixer) = setup();
        play(&mut tx, &constant(200, 0.4, -0.2), 0.5);
        let out = render(&mut mixer, 200);
        assert_eq!(&out[200..202], [0.1, -0.05]);
        for i in 0..200 {
            let g = envelope(i, 200) * 0.25;
            assert_eq!(&out[2 * i..2 * i + 2], [0.4 * g, -0.2 * g], "frame {i}");
        }
    }

    #[test]
    fn fade_ramps_64_frames_equally_in_both_channels() {
        let (mut tx, mut mixer) = setup();
        play(&mut tx, &constant(300, 1.0, -1.0), 1.0);
        let out = render(&mut mixer, 300);
        let gains: Vec<f32> = out.chunks(2).map(|f| f[0] / 0.5).collect();
        for (i, f) in out.chunks(2).enumerate() {
            assert_eq!(f[0], -f[1], "frame {i}");
        }
        assert_eq!(gains[0], 0.0);
        assert_eq!(gains[32], 0.5);
        assert_eq!(gains[63], 63.0 / 64.0);
        assert_eq!(gains[64], 1.0);
        assert_eq!(gains[235], 1.0);
        assert_eq!(gains[236], 63.0 / 64.0);
        assert_eq!(gains[299], 0.0);
    }

    #[test]
    fn short_sound_envelope_is_min_of_fades() {
        let (mut tx, mut mixer) = setup();
        play(&mut tx, &constant(100, 1.0, 1.0), 1.0);
        let out = render(&mut mixer, 100);
        for i in 0..100 {
            assert_eq!(out[2 * i], envelope(i, 100) * 0.5, "frame {i}");
        }
        assert_eq!(out[2 * 49], 49.0 / 128.0);
    }

    #[test]
    fn envelope_continues_across_renders() {
        let (mut tx, mut whole) = setup();
        play(&mut tx, &ramp(200), 1.0);
        let (mut tx, mut parts) = setup();
        play(&mut tx, &ramp(200), 1.0);
        let mut joined = render(&mut parts, 30);
        joined.extend(render(&mut parts, 170));
        assert_eq!(render(&mut whole, 200), joined);
    }

    #[test]
    fn voices_sum_and_clip() {
        let (mut tx, mut mixer) = setup();
        play(&mut tx, &constant(200, 0.4, -0.4), 1.0);
        play(&mut tx, &constant(200, 0.2, -0.2), 1.0);
        let out = render(&mut mixer, 100);
        assert!((out[198] - 0.3).abs() < 1e-6);
        assert!((out[199] + 0.3).abs() < 1e-6);
        for _ in 0..4 {
            play(&mut tx, &constant(200, 1.0, -1.0), 1.0);
        }
        let out = render(&mut mixer, 100);
        assert_eq!(&out[198..200], [1.0, -1.0]);
    }

    #[test]
    fn finished_voice_frees_its_slot() {
        let (mut tx, mut mixer) = setup();
        play(&mut tx, &constant(10, 1.0, 1.0), 1.0);
        play(&mut tx, &constant(11, 1.0, 1.0), 1.0);
        render(&mut mixer, 10);
        assert!(mixer.voices[0].is_none());
        assert!(mixer.voices[1].is_some());
        render(&mut mixer, 10);
        assert!(mixer.voices.iter().all(Option::is_none));
        assert_eq!(render(&mut mixer, 4), [0.0; 8]);
    }

    #[test]
    fn music_loops_with_fade_in_only_on_first_start() {
        let (mut tx, mut mixer) = setup();
        let sound = ramp(100);
        music(&mut tx, &sound, 1.0);
        let mut out = render(&mut mixer, 150);
        out.extend(render(&mut mixer, 150));
        for i in 0..300 {
            let gain = i.min(FADE) as f32 / FADE as f32 * 0.5;
            let j = i % 100;
            let expected = [sound[2 * j] * gain, sound[2 * j + 1] * gain];
            assert_eq!(&out[2 * i..2 * i + 2], expected, "frame {i}");
        }
        assert!(mixer.music.is_some());
    }

    #[test]
    fn empty_sound_does_not_start() {
        let (mut tx, mut mixer) = setup();
        let empty: Arc<[f32]> = Arc::new([]);
        play(&mut tx, &empty, 1.0);
        music(&mut tx, &empty, 1.0);
        render(&mut mixer, 4);
        assert!(mixer.voices.iter().all(Option::is_none));
        assert!(mixer.music.is_none());

        music(&mut tx, &ramp(100), 1.0);
        music(&mut tx, &empty, 1.0);
        render(&mut mixer, 4);
        assert!(mixer.music.is_some());
    }

    #[test]
    fn effect_beyond_voices_is_dropped_and_music_keeps_its_slot() {
        let (mut tx, mut mixer) = setup();
        let sound = constant(100, 0.1, 0.1);
        music(&mut tx, &sound, 1.0);
        for _ in 0..=VOICES {
            play(&mut tx, &sound, 1.0);
        }
        render(&mut mixer, 4);
        assert!(mixer.voices.iter().all(Option::is_some));
        assert!(mixer.music.is_some());
        assert_eq!(mixer.counters.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn effect_on_full_pool_does_not_steal_a_playing_voice() {
        let (mut tx, mut mixer) = setup();
        let sound = constant(1024, 1.0 / 64.0, 1.0 / 64.0);
        for _ in 0..VOICES {
            play(&mut tx, &sound, 1.0);
        }
        render(&mut mixer, 128);
        play(&mut tx, &sound, 1.0);
        assert_eq!(render(&mut mixer, 128), [0.25; 256]);
        assert_eq!(mixer.counters.dropped.load(Ordering::Relaxed), 1);
    }

    fn loop_start(tx: &mut Producer<Command>, slot: usize, generation: u32, frames: &Arc<[f32]>) {
        let frames = frames.clone();
        let command = Command::LoopStart {
            slot,
            generation,
            frames,
            volume: 1.0,
        };
        assert!(tx.push(command).is_ok());
    }

    fn loop_set(tx: &mut Producer<Command>, slot: usize, generation: u32, rate: f32, volume: f32) {
        let command = Command::LoopSet {
            slot,
            generation,
            rate,
            volume,
        };
        assert!(tx.push(command).is_ok());
    }

    fn loop_stop(tx: &mut Producer<Command>, slot: usize, generation: u32) {
        assert!(tx.push(Command::LoopStop { slot, generation }).is_ok());
    }

    /// Голос поколения 1 в слоте 0 на скорости `rate` после fade-in; фаза — `rate · (FADE + 1) mod len`.
    fn steady_loop(frames: &Arc<[f32]>, rate: f32) -> (Producer<Command>, Mixer) {
        let (mut tx, mut mixer) = setup();
        loop_start(&mut tx, 0, 1, frames);
        loop_set(&mut tx, 0, 1, rate, 1.0);
        render(&mut mixer, 1);
        render(&mut mixer, FADE);
        (tx, mixer)
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.iter().step_by(2).copied().collect()
    }

    fn assert_near(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() < 1e-6, "frame {i}: {a} != {e}");
        }
    }

    #[test]
    fn loop_at_rate_two_runs_cycle_twice_as_fast() {
        let sound = ramp(100);
        let (_tx, mut mixer) = steady_loop(&sound, 2.0);
        let out = render(&mut mixer, 100);
        for k in 0..100 {
            let j = (30 + 2 * k) % 100;
            let expected = [sound[2 * j] * 0.5, sound[2 * j + 1] * 0.5];
            assert_eq!(&out[2 * k..2 * k + 2], expected, "frame {k}");
        }
    }

    #[test]
    fn loop_interpolates_fractional_phase_and_stitches_last_frame_to_first() {
        let sound: Arc<[f32]> = [0.1, 0.2, 0.3, 0.9].iter().flat_map(|&s| [s, -s]).collect();
        let (_tx, mut mixer) = steady_loop(&sound, 0.5);
        let out = render(&mut mixer, 8);
        let expected = [0.15, 0.2, 0.25, 0.3, 0.6, 0.9, 0.5, 0.1].map(|s| s * 0.5);
        assert_near(&left(&out), &expected);
        let right: Vec<f32> = out.iter().skip(1).step_by(2).map(|s| -s).collect();
        assert_near(&right, &expected);
    }

    #[test]
    fn loop_volume_ramps_linearly_across_buffer() {
        let (mut tx, mut mixer) = steady_loop(&constant(10, 1.0, -1.0), 1.0);
        loop_set(&mut tx, 0, 1, 1.0, 0.0);
        let out = render(&mut mixer, 100);
        let expected: Vec<f32> = (0..100)
            .map(|k| 0.5 - 0.5 * (k + 1) as f32 / 100.0)
            .collect();
        assert_near(&left(&out), &expected);
        assert_eq!(render(&mut mixer, 10), [0.0; 20]);
        assert!(mixer.loops[0].is_some());
    }

    #[test]
    fn loop_rate_ramps_linearly_across_buffer() {
        let (mut tx, mut mixer) = steady_loop(&ramp(1000), 1.0);
        loop_set(&mut tx, 0, 1, 3.0, 1.0);
        let mut out = left(&render(&mut mixer, 100));
        out.extend(left(&render(&mut mixer, 10)));
        // Звук линеен по фазе: шаг выхода — скорость × 0,5 / 1000.
        for (k, step) in out.windows(2).map(|w| (w[1] - w[0]) / 0.0005).enumerate() {
            let rate = 1.0 + 2.0 * ((k + 1) as f32 / 100.0).min(1.0);
            assert!((step - rate).abs() < 1e-2, "frame {k}: {step} != {rate}");
        }
    }

    #[test]
    fn loop_fades_in_on_start_and_frees_slot_after_stop_fade() {
        let (mut tx, mut mixer) = setup();
        loop_start(&mut tx, 0, 1, &constant(10, 1.0, -1.0));
        let out = render(&mut mixer, FADE);
        let fade_in: Vec<f32> = (0..FADE).map(|k| k as f32 / FADE as f32 * 0.5).collect();
        assert_eq!(left(&out), fade_in);

        loop_stop(&mut tx, 0, 1);
        let out = render(&mut mixer, FADE - 1);
        let fade_out: Vec<f32> = (1..FADE)
            .rev()
            .map(|k| k as f32 / FADE as f32 * 0.5)
            .collect();
        assert_eq!(left(&out), fade_out);
        assert!(mixer.loops[0].is_some());
        assert_eq!(render(&mut mixer, 1), [0.0, 0.0]);
        assert!(mixer.loops[0].is_none());
        assert_eq!(render(&mut mixer, 4), [0.0; 8]);
    }

    #[test]
    fn loop_commands_of_other_generation_or_slot_are_ignored() {
        let (mut tx, mut mixer) = steady_loop(&constant(10, 1.0, -1.0), 1.0);
        loop_set(&mut tx, 0, 2, 1.0, 0.0);
        loop_stop(&mut tx, 0, 2);
        loop_set(&mut tx, 1, 1, 1.0, 0.0);
        loop_stop(&mut tx, LOOPS, 1);
        loop_set(&mut tx, LOOPS, 1, 1.0, 0.0);
        assert_eq!(render(&mut mixer, 100), [0.5, -0.5].repeat(100));
        assert!(mixer.loops[0].is_some());
        assert!(mixer.loops[1..].iter().all(Option::is_none));
    }

    #[test]
    fn loop_start_into_taken_slot_replaces_voice() {
        let (mut tx, mut mixer) = steady_loop(&constant(10, 1.0, -1.0), 1.0);
        let empty: Arc<[f32]> = Arc::new([]);
        loop_start(&mut tx, 0, 2, &empty);
        loop_start(&mut tx, LOOPS, 2, &constant(10, 1.0, -1.0));
        assert_eq!(render(&mut mixer, 2), [0.5, -0.5, 0.5, -0.5]);

        loop_start(&mut tx, 0, 3, &constant(10, 0.2, -0.2));
        loop_set(&mut tx, 0, 1, 1.0, 0.0);
        assert_eq!(render(&mut mixer, 1), [0.0, 0.0]);
        render(&mut mixer, FADE);
        assert_eq!(render(&mut mixer, 2), [0.1, -0.1, 0.1, -0.1]);
        assert_eq!(mixer.loops[0].as_ref().map(|v| v.generation), Some(3));
    }

    #[test]
    fn loop_rate_is_clamped_to_zero_and_max() {
        let (mut tx, mut mixer) = setup();
        loop_start(&mut tx, 0, 1, &ramp(10));
        for (rate, expected) in [(10.0, MAX_RATE), (-1.0, 0.0), (f32::NAN, 0.0), (2.5, 2.5)] {
            loop_set(&mut tx, 0, 1, rate, 1.0);
            render(&mut mixer, 1);
            assert_eq!(
                mixer.loops[0].as_ref().map(|v| v.rate),
                Some(expected),
                "{rate}"
            );
        }
    }

    #[test]
    fn render_takes_at_most_64_commands() {
        let (mut tx, mut mixer) = setup();
        let empty: Arc<[f32]> = Arc::new([]);
        for _ in 0..=COMMANDS_PER_RENDER {
            play(&mut tx, &empty, 1.0);
        }
        render(&mut mixer, 4);
        assert_eq!(mixer.queue.slots(), 1);
        render(&mut mixer, 4);
        assert_eq!(mixer.queue.slots(), 0);
    }
}
