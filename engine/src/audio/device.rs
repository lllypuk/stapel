use std::sync::Arc;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BuildStreamError, Device, SampleFormat, SampleRate, SizedSample, Stream, StreamConfig};

use super::mixer::{Counters, Mixer};
use super::{Audio, DeviceSample, fill};

const RATE: u32 = 48_000;
/// Стерео-фреймов в `scratch`; буфер устройства длиннее микшируется кусками.
const SCRATCH_FRAMES: usize = 2048;

/// Запущенный выход по умолчанию, `Audio` к нему и частота; нет устройства или формата — `warn` и `None`.
pub(crate) fn open() -> Option<(Stream, Audio, u32)> {
    let Some(device) = cpal::default_host().default_output_device() else {
        log::warn!("audio: no output device, muted");
        return None;
    };
    let preferred = device
        .supported_output_configs()
        .ok()
        .and_then(|mut configs| {
            configs
                .find(|c| {
                    c.channels() == 2
                        && c.sample_format() == SampleFormat::F32
                        && (c.min_sample_rate().0..=c.max_sample_rate().0).contains(&RATE)
                })
                .map(|c| c.with_sample_rate(SampleRate(RATE)))
        });
    let supported = match preferred.map_or_else(|| device.default_output_config(), Ok) {
        Ok(supported) => supported,
        Err(e) => {
            log::warn!("audio: {e}, muted");
            return None;
        }
    };
    let config = supported.config();
    let (audio, mixer) = Audio::open();
    let counters = audio.counters.clone();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, mixer, counters),
        SampleFormat::I16 => build::<i16>(&device, &config, mixer, counters),
        other => {
            log::warn!("audio: sample format {other} unsupported, muted");
            return None;
        }
    };
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!("audio: {e}, muted");
            return None;
        }
    };
    if let Err(e) = stream.play() {
        log::warn!("audio: {e}, muted");
        return None;
    }
    log::info!(
        "audio: {} Hz, {} ch, {}",
        config.sample_rate.0,
        config.channels,
        supported.sample_format()
    );
    Some((stream, audio, config.sample_rate.0))
}

fn build<S: DeviceSample + SizedSample>(
    device: &Device,
    config: &StreamConfig,
    mut mixer: Mixer,
    counters: Arc<Counters>,
) -> Result<Stream, BuildStreamError> {
    let channels = usize::from(config.channels);
    let rate = config.sample_rate.0;
    let mut scratch = vec![0.0; SCRATCH_FRAMES * 2];
    device.build_output_stream(
        config,
        move |out: &mut [S], _: &_| {
            let start = Instant::now();
            fill(&mut mixer, &mut scratch, out, channels);
            counters.record_callback(start.elapsed(), out.len() / channels, rate);
        },
        // Аллоцирует на аудиопотоке, но только на пути ошибки.
        |e| log::error!("audio stream: {e}"),
        None,
    )
}
