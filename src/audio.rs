use anyhow::{Context, Result, bail, ensure};
use rubato::{FftFixedIn, Resampler};
use std::{
    fs::File,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub const SAMPLE_RATE: u32 = 16_000;
const MAX_SAMPLES: usize = SAMPLE_RATE as usize * 4 * 3600;
const BLOCK: usize = 4096;

#[derive(Clone)]
pub struct Audio {
    pub samples: Arc<[f32]>,
    pub peaks: Arc<[f32]>,
}

impl Audio {
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / SAMPLE_RATE as f64
    }
}

pub fn decode(path: &Path, cancel: &AtomicBool, mut progress: impl FnMut(f32)) -> Result<Audio> {
    if let Some(audio) = decode_opus(path, cancel, &mut progress)? {
        return Ok(audio);
    }
    let input = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            input,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .context("Unsupported format. Try WAV, MP3, FLAC, M4A or MP4 with AAC.")?
        .format;
    let (track_id, params, mut decoder) = format
        .tracks()
        .iter()
        .find_map(|track| {
            symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .ok()
                .map(|decoder| (track.id, track.codec_params.clone(), decoder))
        })
        .context("No supported audio track (WebM/Opus, WMA and AC-3 are not supported yet).")?;
    let mut resampler: Option<FftFixedIn<f32>> = None;
    let mut rate = 0;
    let mut pending = Vec::with_capacity(BLOCK * 2);
    let mut samples = Vec::new();
    let mut input_frames: usize = 0;
    let mut packets = 0;
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error).context("Could not read recording"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .context("Damaged or unsupported audio track")?;
        let spec = *decoded.spec();
        if rate == 0 {
            rate = spec.rate;
            ensure!(rate > 0 && rate <= 384_000, "Invalid sample rate");
            if rate != SAMPLE_RATE {
                resampler = Some(FftFixedIn::new(
                    rate as usize,
                    SAMPLE_RATE as usize,
                    BLOCK,
                    2,
                    1,
                )?);
            }
        }
        ensure!(
            rate == spec.rate,
            "Sample-rate changes within a recording are not supported"
        );
        let channels = spec.channels.count();
        ensure!(channels > 0, "Audio track has no channels");
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            pending.push(if mono.is_finite() {
                mono.clamp(-1.0, 1.0)
            } else {
                0.0
            });
            input_frames += 1;
        }
        if let Some(resampler) = &mut resampler {
            while pending.len() >= BLOCK {
                let output = resampler.process(&[&pending[..BLOCK]], None)?;
                samples.extend_from_slice(&output[0]);
                pending.drain(..BLOCK);
            }
        } else {
            samples.append(&mut pending);
        }
        ensure!(
            samples.len() <= MAX_SAMPLES,
            "Maximum recording length is 4 hours"
        );
        packets += 1;
        if packets % 64 == 0 {
            let p = params
                .n_frames
                .filter(|n| *n > 0)
                .map_or(0.0, |n| input_frames as f32 / n as f32);
            progress(p.clamp(0.0, 1.0));
        }
    }
    if input_frames == 0 {
        bail!("Audio track is empty");
    }
    if let Some(mut resampler) = resampler {
        let delay = resampler.output_delay();
        pending.resize(BLOCK, 0.0);
        samples.extend_from_slice(&resampler.process(&[pending], None)?[0]);
        let expected = (input_frames as f64 * SAMPLE_RATE as f64 / rate as f64).round() as usize;
        while samples.len() < expected + delay {
            samples.extend_from_slice(&resampler.process(&[vec![0.0; BLOCK]], None)?[0]);
        }
        samples.drain(..delay);
        samples.truncate(expected);
    }
    ensure!(
        samples.len() <= MAX_SAMPLES,
        "Maximum recording length is 4 hours"
    );
    ensure!(!samples.is_empty(), "Audio track is too short");
    progress(1.0);
    Ok(from_samples(samples))
}

fn from_samples(samples: Vec<f32>) -> Audio {
    let peaks = samples
        .chunks(samples.len().div_ceil(1600))
        .map(|chunk| {
            chunk
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        })
        .collect::<Vec<_>>();
    Audio {
        samples: samples.into(),
        peaks: peaks.into(),
    }
}

// Most messenger voice notes use Ogg Opus, which Symphonia does not decode.
// libopus is linked statically, so voice notes do not need an external tool.
fn decode_opus(
    path: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f32),
) -> Result<Option<Audio>> {
    use std::io::{Read, Seek};
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    if file.read(&mut magic)? != 4 || &magic != b"OggS" {
        return Ok(None);
    }
    file.rewind()?;
    let size = file.metadata()?.len().max(1);
    let mut reader = ogg::reading::PacketReader::new(file);
    let Some(header) = reader.read_packet()? else {
        return Ok(None);
    };
    if !header.data.starts_with(b"OpusHead") {
        return Ok(None);
    }
    ensure!(header.data.len() >= 19, "Damaged Opus header");
    ensure!(
        header.data[8] <= 15 && header.data[18] == 0,
        "This Opus variant is not supported"
    );
    let channels = match header.data[9] {
        1 => opus::Channels::Mono,
        2 => opus::Channels::Stereo,
        _ => bail!("Only mono and stereo Opus are supported"),
    };
    let count = header.data[9] as usize;
    let pre_skip = u16::from_le_bytes([header.data[10], header.data[11]]) as usize;
    let gain_db = i16::from_le_bytes([header.data[16], header.data[17]]) as f32 / 256.0;
    let gain = 10.0_f32.powf(gain_db / 20.0);
    let serial = header.stream_serial();
    let tags = reader.read_packet_expected()?;
    ensure!(
        tags.stream_serial() == serial && tags.data.starts_with(b"OpusTags"),
        "Damaged Opus metadata"
    );
    let mut decoder = opus::Decoder::new(SAMPLE_RATE, channels)?;
    let mut buffer = vec![0.0_f32; 1920 * count];
    let mut samples = Vec::new();
    let mut final_position = None;
    let mut packets = 0;
    while let Some(packet) = reader.read_packet()? {
        ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
        ensure!(
            packet.stream_serial() == serial && final_position.is_none(),
            "Chained Ogg streams are not supported yet"
        );
        let frames = decoder
            .decode_float(&packet.data, &mut buffer, false)
            .context("Damaged Opus packet")?;
        samples.extend(
            buffer[..frames * count]
                .chunks_exact(count)
                .map(|frame| (frame.iter().sum::<f32>() / count as f32 * gain).clamp(-1.0, 1.0)),
        );
        ensure!(
            samples.len() <= MAX_SAMPLES + 1920,
            "Maximum recording length is 4 hours"
        );
        if packet.last_in_stream() {
            final_position = Some(packet.absgp_page());
        }
        packets += 1;
        if packets % 64 == 0 {
            progress(reader.get_mut().stream_position()? as f32 / size as f32);
        }
    }
    let final_position =
        final_position.context("Truncated Opus recording: missing final packet")?;
    ensure!(final_position >= pre_skip as u64, "Invalid Opus duration");
    // Opus granule positions and pre-skip always use a 48 kHz clock.
    let skip = pre_skip.div_ceil(3);
    ensure!(samples.len() > skip, "Audio track is empty");
    samples.drain(..skip);
    let expected = ((final_position - pre_skip as u64) / 3) as usize;
    ensure!(
        expected > 0 && expected <= samples.len(),
        "Invalid Opus duration"
    );
    samples.truncate(expected);
    progress(1.0);
    Ok(Some(from_samples(samples)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(rate: u32, channels: u16, seconds: f32) -> Vec<u8> {
        let frames = (rate as f32 * seconds) as usize;
        let size = frames as u32 * channels as u32 * 2;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * channels as u32 * 2).to_le_bytes());
        bytes.extend((channels * 2).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for i in 0..frames {
            let sample =
                ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16;
            for _ in 0..channels {
                bytes.extend(sample.to_le_bytes());
            }
        }
        bytes
    }
    #[test]
    fn stereo_resampling_preserves_duration_and_signal() {
        let directory = tempfile::tempdir().unwrap();
        for rate in [8_000, 16_000, 44_100, 48_000] {
            let path = directory.path().join(format!("audio-é-{rate}.wav"));
            std::fs::write(&path, wav(rate, 2, 0.37)).unwrap();
            let audio = decode(&path, &AtomicBool::new(false), |_| {}).unwrap();
            assert!((audio.duration() - 0.37).abs() < 0.001);
            assert!(audio.samples.iter().all(|s| s.is_finite()));
            let rms = (audio.samples.iter().map(|s| s * s).sum::<f32>()
                / audio.samples.len() as f32)
                .sqrt();
            assert!((rms - 0.345).abs() < 0.02, "{rate}: {rms}");
        }
    }
    #[test]
    fn cancellation_stops_decoding() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.wav");
        std::fs::write(&path, wav(16_000, 1, 0.1)).unwrap();
        assert!(decode(&path, &AtomicBool::new(true), |_| {}).is_err());
    }

    #[test]
    fn decodes_messenger_opus_and_trims_end_padding() {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("voice-message.ogg");
        let file = File::create(&path).unwrap();
        let mut writer = PacketWriter::new(file);
        let mut head = b"OpusHead".to_vec();
        head.extend([1, 1]); // version, mono
        head.extend(0u16.to_le_bytes()); // pre-skip
        head.extend(16_000u32.to_le_bytes());
        head.extend([0, 0, 0]); // gain, channel mapping
        writer
            .write_packet(head, 1, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        let mut tags = b"OpusTags".to_vec();
        tags.extend(0u32.to_le_bytes());
        tags.extend(0u32.to_le_bytes());
        writer
            .write_packet(tags, 1, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        let mut encoder =
            opus::Encoder::new(16_000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        let mut buffer = vec![0u8; 4000];
        for i in 0..5 {
            let pcm = (0..320)
                .map(|j| {
                    ((i * 320 + j) as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.4
                })
                .collect::<Vec<_>>();
            let size = encoder.encode_float(&pcm, &mut buffer).unwrap();
            let end = if i == 4 {
                PacketWriteEndInfo::EndStream
            } else {
                PacketWriteEndInfo::EndPage
            };
            // Last packet contains 5ms of padding.
            let granule = if i == 4 { 4560 } else { (i + 1) * 960 };
            writer
                .write_packet(buffer[..size].to_vec(), 1, end, granule as u64)
                .unwrap();
        }
        drop(writer);
        let audio = decode(&path, &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(audio.samples.len(), 1520);
        assert!(audio.samples.iter().any(|s| s.abs() > 0.1));
        assert!(decode(&path, &AtomicBool::new(true), |_| {}).is_err());
    }
}
