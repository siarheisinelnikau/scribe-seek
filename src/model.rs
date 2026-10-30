use crate::storage::Storage;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Copy)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const MODELS: [Model; 3] = [
    Model {
        id: "tiny",
        label: "Tiny · faster · 74 MiB",
        size: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        id: "base",
        label: "Base · balanced · 141 MiB",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        id: "small-q5_1",
        label: "Small · more accurate · 181 MiB",
        size: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
];

impl Model {
    pub fn installed(&self, storage: &Storage) -> bool {
        fs::metadata(storage.model_path(self.id)).is_ok_and(|m| m.len() == self.size)
    }
    pub fn verify(&self, path: &Path, cancel: &AtomicBool) -> Result<()> {
        let mut file = fs::File::open(path).context("Download a model first")?;
        ensure!(
            file.metadata()?.len() == self.size,
            "Invalid model size. Download it again."
        );
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == self.sha256,
            "Invalid model checksum. Download it again."
        );
        Ok(())
    }
}

pub fn download(
    model: Model,
    storage: &Storage,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    let path = storage.model_path(model.id);
    let temp = path.with_extension(format!("{}.partial", std::process::id()));
    let result = (|| -> Result<()> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(3600)))
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_per_read(Some(Duration::from_secs(10)))
            .build()
            .new_agent();
        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
            model.id
        );
        let mut response = agent
            .get(&url)
            .call()
            .context("Could not download model from Hugging Face")?;
        let mut input = response.body_mut().as_reader();
        let mut output = fs::File::create(&temp)?;
        let mut buffer = vec![0u8; 128 * 1024];
        let mut received = 0u64;
        let mut hash = Sha256::new();
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received += count as u64;
            ensure!(
                received <= model.size,
                "Server returned an invalid model size"
            );
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            progress(received as f32 / model.size as f32);
        }
        ensure!(
            received == model.size,
            "Model download was interrupted. Try again."
        );
        ensure!(
            format!("{:x}", hash.finalize()) == model.sha256,
            "Model checksum does not match"
        );
        ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
        output.sync_all()?;
        drop(output);
        fs::rename(&temp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
