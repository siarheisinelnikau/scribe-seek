use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcript {
    pub version: u32,
    pub source: PathBuf,
    pub source_size: u64,
    pub source_modified_ms: u64,
    pub duration: f64,
    pub model: String,
    pub language: String,
    pub segments: Vec<Segment>,
}

impl Transcript {
    pub fn title(&self) -> String {
        self.source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported transcript version");
        ensure!(
            self.duration.is_finite() && self.duration > 0.0,
            "Invalid duration"
        );
        let mut previous_start = 0.0;
        for segment in &self.segments {
            ensure!(
                segment.start.is_finite() && segment.end.is_finite(),
                "Invalid timestamps"
            );
            ensure!(
                segment.start >= previous_start && segment.end >= segment.start,
                "Timestamps are out of order"
            );
            ensure!(
                segment.end <= self.duration + 0.1,
                "Timestamp is outside the recording"
            );
            previous_start = segment.start;
        }
        Ok(())
    }

    pub fn search(&self, query: &str) -> Vec<usize> {
        let query = normalize(query.trim());
        if query.is_empty() {
            return Vec::new();
        }
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, s)| normalize(&s.text).contains(&query))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn srt(&self) -> String {
        self.segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{}\n{} --> {}\n{}\n\n",
                    i + 1,
                    timestamp(s.start, ','),
                    timestamp(s.end, ','),
                    s.text.trim()
                )
            })
            .collect()
    }

    pub fn vtt(&self) -> String {
        let mut output = String::from("WEBVTT\n\n");
        for s in &self.segments {
            output.push_str(&format!(
                "{} --> {}\n{}\n\n",
                timestamp(s.start, '.'),
                timestamp(s.end, '.'),
                s.text.trim()
            ));
        }
        output
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase().replace('\u{451}', "\u{435}")
}

pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

fn timestamp(seconds: f64, separator: char) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        separator,
        ms % 1000
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn fixture() -> Transcript {
        Transcript {
            version: 1,
            source: "lecture-é.mp4".into(),
            source_size: 100,
            source_modified_ms: 0,
            duration: 62.0,
            model: "tiny".into(),
            language: "en".into(),
            segments: vec![
                Segment {
                    start: 0.0,
                    end: 2.5,
                    text: "Résumé of the LECTURE.".into(),
                },
                Segment {
                    start: 59.9996,
                    end: 61.0,
                    text: "Find the phrase.".into(),
                },
            ],
        }
    }
    #[test]
    fn unicode_search_and_empty_query() {
        let t = fixture();
        assert_eq!(t.search("  résumé OF "), vec![0]);
        assert_eq!(t.search("PHRASE"), vec![1]);
        assert!(t.search("  ").is_empty());
    }
    #[test]
    fn subtitle_timestamps_carry_across_minutes() {
        assert!(fixture().srt().contains("2\n00:01:00,000 --> 00:01:01,000"));
        assert!(fixture().vtt().starts_with("WEBVTT\n\n00:00:00.000"));
    }
    #[test]
    fn rejects_invalid_or_unordered_times() {
        let mut t = fixture();
        t.validate().unwrap();
        t.segments[1].start = f64::NAN;
        assert!(t.validate().is_err());
        t.segments[1].start = 0.0;
        t.segments[0].start = 1.0;
        assert!(t.validate().is_err());
    }
}
