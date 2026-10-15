use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;

/// Отсчёты-интерлив, число каналов, частота файла.
type Decoded = (Vec<f32>, u16, u32);

/// Стерео-интерлив в частоте `rate`: WAV PCM 16 бит или OGG Vorbis, моно или стерео.
pub(crate) fn load(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let (samples, channels, from) = match path.extension().and_then(|e| e.to_str()) {
        Some("wav") => read_wav(path)?,
        Some("ogg") => read_ogg(path)?,
        _ => return Err("expected .wav or .ogg".into()),
    };
    if from == 0 {
        return Err("sample rate 0".into());
    }
    let stereo = match channels {
        1 => samples.iter().flat_map(|&s| [s, s]).collect(),
        2 => samples,
        n => return Err(format!("{n} channels, expected 1 or 2")),
    };
    Ok(resample(&stereo, from, rate))
}

fn read_wav(path: &Path) -> Result<Decoded, String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    // 8 бит hound читает как i16 без ошибки — в 256 раз тише.
    if spec.bits_per_sample != 16 {
        return Err(format!("{} bit, expected 16", spec.bits_per_sample));
    }
    let samples = reader
        .into_samples::<i16>()
        .map(|s| s.map(|s| f32::from(s) / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok((samples, spec.channels, spec.sample_rate))
}

fn read_ogg(path: &Path) -> Result<Decoded, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut ogg = OggStreamReader::new(BufReader::new(file)).map_err(|e| e.to_string())?;
    let mut samples = Vec::new();
    while let Some(packet) = ogg
        .read_dec_packet_generic::<InterleavedSamples<f32>>()
        .map_err(|e| e.to_string())?
    {
        samples.extend_from_slice(&packet.samples);
    }
    let header = &ogg.ident_hdr;
    Ok((
        samples,
        u16::from(header.audio_channels),
        header.audio_sample_rate,
    ))
}

/// Линейная интерполяция стерео-интерлива; за последним фреймом — он сам.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    let frames = samples.as_chunks::<2>().0;
    let (from, to) = (u64::from(from), u64::from(to));
    let len = (frames.len() as u64 * to).div_ceil(from);
    let mut out = Vec::new();
    for i in 0..len {
        let pos = i * from;
        #[expect(clippy::cast_possible_truncation, reason = "pos / to < frames.len()")]
        let j = (pos / to) as usize;
        let (a, b) = (frames[j], frames.get(j + 1).unwrap_or(&frames[j]));
        let t = (pos % to) as f32 / to as f32;
        out.extend([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("engine-decode-{}-{name}", std::process::id()))
    }

    fn load_wav(name: &str, channels: u16, bits: u16, samples: &[i32]) -> Result<Vec<f32>, String> {
        let path = temp_path(name);
        let spec = hound::WavSpec {
            channels,
            sample_rate: 48_000,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
        let out = load(&path, 48_000);
        std::fs::remove_file(&path).unwrap();
        out
    }

    #[test]
    fn wav_mono_becomes_stereo() {
        let out = load_wav("mono.wav", 1, 16, &[0, 16384, -32768]).unwrap();
        assert_eq!(out, [0.0, 0.0, 0.5, 0.5, -1.0, -1.0]);
    }

    #[test]
    fn wav_stereo_is_kept() {
        let out = load_wav("stereo.wav", 2, 16, &[8192, -8192, 32767, 0]).unwrap();
        assert_eq!(out, [0.25, -0.25, 32767.0 / 32768.0, 0.0]);
    }

    #[test]
    fn wav_not_16_bit_or_stereo_is_rejected() {
        assert!(load_wav("8bit.wav", 1, 8, &[0, 100]).is_err());
        assert!(load_wav("24bit.wav", 1, 24, &[0, 100]).is_err());
        assert!(load_wav("6ch.wav", 6, 16, &[0; 12]).is_err());
    }

    #[test]
    fn ogg_mono_sine_decodes_to_equal_channels() {
        // ffmpeg -f lavfi -i sine=440:d=0.1 -c:a libvorbis engine/tests/data/sine.ogg
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sine.ogg"));
        let out = load(path, 44_100).unwrap();
        let frames = out.as_chunks::<2>().0;
        assert!(
            (4410..4410 + 1024).contains(&frames.len()),
            "{}",
            frames.len()
        );
        assert!(frames.iter().all(|[l, r]| l.to_bits() == r.to_bits()));
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((0.1..0.15).contains(&peak), "{peak}");
    }

    #[test]
    fn missing_broken_or_unknown_file_is_rejected() {
        assert!(load(&temp_path("missing.wav"), 48_000).is_err());
        assert!(load(&temp_path("missing.ogg"), 48_000).is_err());
        let path = temp_path("broken.ogg");
        std::fs::write(&path, b"OggS not really").unwrap();
        assert!(load(&path, 48_000).is_err());
        std::fs::remove_file(&path).unwrap();
        let path = temp_path("sound.mp3");
        std::fs::write(&path, b"").unwrap();
        assert!(load(&path, 48_000).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn zero_sample_rate_is_rejected() {
        let path = temp_path("zero-rate.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        w.write_sample(0_i16).unwrap();
        w.finalize().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        // Частота и байт/с вместе: hound сверяет их между собой.
        bytes[24..32].fill(0);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(load(&path, 48_000), Err("sample rate 0".into()));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn resample_44100_to_48000_interpolates_linearly() {
        let ramp: Vec<f32> = (0..441).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let out = resample(&ramp, 44_100, 48_000);
        assert_eq!(out.len(), 480 * 2);
        for (i, [l, r]) in out.as_chunks::<2>().0.iter().enumerate() {
            let want = (i as f32 * 0.918_75).min(440.0);
            assert!(
                (l - want).abs() < 1e-3 && (r + want).abs() < 1e-3,
                "{i}: {l} {r}"
            );
        }
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let samples = [0.1, -0.2, 0.3, -0.4, 0.5, -0.6];
        assert_eq!(resample(&samples, 48_000, 48_000), samples);
    }
}
