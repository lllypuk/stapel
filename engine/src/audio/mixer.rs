use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rtrb::Consumer;

/// Голосов эффектов; музыка — отдельный слот сверх них.
const VOICES: usize = 32;
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

/// Микс голосов аудиопотока; команды берёт из очереди `Audio`.
pub(crate) struct Mixer {
    queue: Consumer<Command>,
    voices: [Option<Voice>; VOICES],
    music: Option<Voice>,
    counters: Arc<Counters>,
}

impl Mixer {
    pub(crate) fn new(queue: Consumer<Command>, counters: Arc<Counters>) -> Self {
        Self {
            queue,
            voices: [const { None }; VOICES],
            music: None,
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
