use crate::{
    audio::{self, Audio},
    model::{self, Model},
    storage::{self, Storage},
    transcribe,
    transcript::Transcript,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
};

pub enum Task {
    Decode(PathBuf),
    Download(Model),
    Transcribe {
        source: PathBuf,
        audio: Audio,
        fingerprint: (u64, u64),
        model: Model,
        language: String,
    },
}

pub enum Event {
    Progress(String, f32),
    AudioReady(PathBuf, Audio, (u64, u64)),
    ModelReady,
    TranscriptReady(Transcript),
    Error(String),
    Finished,
}

pub struct Job {
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn spawn(task: Task, storage: Storage) -> Self {
        let (sender, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(task, storage, flag.clone(), &sender)
            }));
            match result {
                Ok(Err(error)) => {
                    let _ = sender.send(Event::Error(if flag.load(Ordering::Relaxed) {
                        "Operation cancelled".into()
                    } else {
                        format!("{error:#}")
                    }));
                }
                Err(_) => {
                    let _ = sender.send(Event::Error(
                        "Internal error while processing the recording".into(),
                    ));
                }
                _ => {}
            }
            let _ = sender.send(Event::Finished);
        });
        Self { events, cancel }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn run(
    task: Task,
    storage: Storage,
    cancel: Arc<AtomicBool>,
    sender: &Sender<Event>,
) -> anyhow::Result<()> {
    match task {
        Task::Decode(path) => {
            let before = storage::fingerprint(&path)?;
            let audio = audio::decode(&path, &cancel, |p| {
                let _ = sender.send(Event::Progress("Reading audio track".into(), p));
            })?;
            anyhow::ensure!(
                before == storage::fingerprint(&path)?,
                "File changed while reading"
            );
            anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
            let _ = sender.send(Event::AudioReady(path, audio, before));
        }
        Task::Download(model) => {
            model::download(model, &storage, &cancel, |p| {
                let _ = sender.send(Event::Progress("Downloading model".into(), p));
            })?;
            let _ = sender.send(Event::ModelReady);
        }
        Task::Transcribe {
            source,
            audio,
            fingerprint,
            model,
            language,
        } => {
            let progress_sender = sender.clone();
            let document = transcribe::transcribe(
                transcribe::Request {
                    audio: &audio,
                    source: &source,
                    expected_fingerprint: fingerprint,
                    model,
                    language: &language,
                },
                &storage,
                cancel,
                move |p| {
                    let _ = progress_sender.send(Event::Progress(
                        "Transcribing on this computer".into(),
                        p as f32 / 100.0,
                    ));
                },
            )?;
            // Return the result before saving so a disk error cannot discard a completed transcription.
            let _ = sender.send(Event::TranscriptReady(document.clone()));
            storage.save(&document)?;
        }
    }
    Ok(())
}
