use crate::audio::{Audio, SAMPLE_RATE};
use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub struct Playback {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    audio: Option<Audio>,
    speed: f32,
}

impl Playback {
    pub fn load(&mut self, audio: Audio) {
        self.sink = None;
        self.audio = Some(audio);
        self.speed = 1.0;
    }

    pub fn clear(&mut self) {
        self.sink = None;
        self.audio = None;
    }

    fn ensure_sink(&mut self) -> Result<&Sink> {
        if self.sink.is_none() {
            if self.stream.is_none() {
                let mut stream = OutputStreamBuilder::open_default_stream()
                    .context("Audio output device is unavailable")?;
                stream.log_on_drop(false);
                self.stream = Some(stream);
            }
            let audio = self.audio.as_ref().context("Open a recording first")?;
            let sink = Sink::connect_new(self.stream.as_ref().unwrap().mixer());
            sink.pause();
            sink.set_speed(self.speed.max(0.5));
            sink.append(SharedSource {
                samples: audio.samples.clone(),
                cursor: 0,
            });
            self.sink = Some(sink);
        }
        Ok(self.sink.as_ref().unwrap())
    }

    pub fn toggle(&mut self) -> Result<()> {
        let at_end = self
            .audio
            .as_ref()
            .is_some_and(|a| self.position() >= a.duration() - 0.03);
        if at_end || self.sink.as_ref().is_some_and(Sink::empty) {
            self.sink = None;
        }
        let sink = self.ensure_sink()?;
        if sink.is_paused() {
            sink.play();
        } else {
            sink.pause();
        }
        Ok(())
    }

    pub fn seek(&mut self, seconds: f64, play: bool) -> Result<()> {
        let end = self.audio.as_ref().map_or(0.0, Audio::duration);
        if self.sink.as_ref().is_some_and(Sink::empty) {
            self.sink = None;
        }
        let sink = self.ensure_sink()?;
        sink.try_seek(Duration::from_secs_f64(seconds.clamp(0.0, end)))
            .map_err(|e| anyhow::anyhow!("Could not seek: {e}"))?;
        if play {
            sink.play();
        }
        Ok(())
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        if let Some(sink) = &self.sink {
            sink.set_speed(speed);
        }
    }
    pub fn position(&self) -> f64 {
        self.sink
            .as_ref()
            .map_or(0.0, |s| s.get_pos().as_secs_f64())
    }
    pub fn playing(&self) -> bool {
        self.sink
            .as_ref()
            .is_some_and(|s| !s.is_paused() && !s.empty())
    }
    pub fn ready(&self) -> bool {
        self.audio.is_some()
    }
}

struct SharedSource {
    samples: Arc<[f32]>,
    cursor: usize,
}
impl Iterator for SharedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.samples.get(self.cursor).copied()?;
        self.cursor += 1;
        Some(sample)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.samples.len().saturating_sub(self.cursor);
        (remaining, Some(remaining))
    }
}
impl Source for SharedSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.cursor))
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.samples.len() as f64 / SAMPLE_RATE as f64,
        ))
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.cursor = (pos.as_secs_f64() * SAMPLE_RATE as f64) as usize;
        self.cursor = self.cursor.min(self.samples.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeks_without_copying_or_restarting_from_zero() {
        let mut source = SharedSource {
            samples: vec![0.0; 32_000].into(),
            cursor: 0,
        };
        source.try_seek(Duration::from_millis(500)).unwrap();
        assert_eq!(source.cursor, 8_000);
        assert_eq!(source.next(), Some(0.0));
        source.try_seek(Duration::from_secs(100)).unwrap();
        assert_eq!(source.next(), None);
    }
}
