//! Exercise the same offline pipeline as the desktop UI without opening a window.
use anyhow::{Context, Result, ensure};
use scribe_seek::{
    audio,
    model::{self, MODELS},
    storage::{Storage, fingerprint},
    transcribe,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source =
        PathBuf::from(args.next().context(
            "Usage: transcribe <media> <data-dir> [tiny|base|small-q5_1] [auto|ru|en|…]",
        )?);
    let storage = Storage::at(PathBuf::from(
        args.next().context("Missing data directory")?,
    ))?;
    let id = args.next().unwrap_or_else(|| "tiny".into());
    let language = args.next().unwrap_or_else(|| "auto".into());
    let model = *MODELS
        .iter()
        .find(|m| m.id == id.to_string_lossy())
        .context("Unknown model")?;
    let source = std::fs::canonicalize(source)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if !model.installed(&storage) {
        eprintln!("Downloading {}…", model.id);
        model::download(model, &storage, &cancel, |_| {})?;
    }
    let before = fingerprint(&source)?;
    let audio = audio::decode(&source, &cancel, |_| {})?;
    eprintln!(
        "Decoded {:.3}s / {} samples",
        audio.duration(),
        audio.samples.len()
    );
    let document = transcribe::transcribe(
        transcribe::Request {
            audio: &audio,
            source: &source,
            expected_fingerprint: before,
            model,
            language: &language.to_string_lossy(),
        },
        &storage,
        cancel,
        |p| eprintln!("Transcription: {p}%"),
    )?;
    storage.save(&document)?;
    ensure!(!document.segments.is_empty(), "No speech recognized");
    println!("{}", document.srt());
    eprintln!("Saved {} segments", document.segments.len());
    Ok(())
}
