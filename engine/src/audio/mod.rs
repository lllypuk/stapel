pub(crate) mod decode;
mod mixer;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rtrb::{Producer, RingBuffer};

use mixer::{Command, Counters, Mixer};

/// Команд в очереди от игры к аудиопотоку.
const QUEUE: usize = 256;

/// Стерео-интерлив f32 в частоте `Assets`; пустой — заглушка ошибки загрузки.
#[derive(Clone, Debug)]
pub struct Sound {
    pub(crate) frames: Arc<[f32]>,
}

/// Запуск звуков из игры; `Default` — немой, команды никуда не идут.
#[derive(Default)]
pub struct Audio {
    queue: Option<Producer<Command>>,
    counters: Arc<Counters>,
}

impl Audio {
    /// Игровая сторона и микшер аудиопотока над общей очередью.
    #[cfg_attr(not(test), expect(dead_code, reason = "открывает устройство, Task 7"))]
    pub(crate) fn open() -> (Self, Mixer) {
        let (producer, consumer) = RingBuffer::new(QUEUE);
        let counters = Arc::new(Counters::default());
        let mixer = Mixer::new(consumer, counters.clone());
        let audio = Self {
            queue: Some(producer),
            counters,
        };
        (audio, mixer)
    }

    /// Эффект поверх остальных; все голоса заняты — отбрасывается.
    pub fn play(&mut self, sound: &Sound, volume: f32) {
        let frames = sound.frames.clone();
        self.send(Command::Play { frames, volume });
    }

    /// Музыка по кругу; играющую обрывает со щелчком — звать из `init`.
    pub fn play_music(&mut self, sound: &Sound, volume: f32) {
        let frames = sound.frames.clone();
        self.send(Command::Music { frames, volume });
    }

    fn send(&mut self, command: Command) {
        if let Some(queue) = &mut self.queue
            && queue.push(command).is_err()
        {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
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
#[cfg_attr(not(test), expect(dead_code, reason = "колбэк устройства, Task 7"))]
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
