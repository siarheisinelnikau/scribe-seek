use crate::{
    audio::Audio,
    model::Model,
    storage::{Storage, fingerprint},
    transcript::{Segment, Transcript},
};
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Request<'a> {
    pub audio: &'a Audio,
    pub source: &'a Path,
    pub expected_fingerprint: (u64, u64),
    pub model: Model,
    pub language: &'a str,
}

pub fn transcribe(
    request: Request<'_>,
    storage: &Storage,
    cancel: Arc<AtomicBool>,
    progress: impl FnMut(i32) + 'static,
) -> Result<Transcript> {
    let Request {
        audio,
        source,
        expected_fingerprint,
        model,
        language,
    } = request;
    ensure!(
        fingerprint(source)? == expected_fingerprint,
        "Source file changed. Open it again."
    );
    let path = storage.model_path(model.id);
    model.verify(&path, &cancel)?;
    let mut context_params = WhisperContextParameters::default();
    context_params.use_gpu(false);
    // Buffer loading also works when the Windows user profile has a non-ASCII name.
    let bytes = std::fs::read(path)?;
    let context = WhisperContext::new_from_buffer_with_params(&bytes, context_params)
        .context("Could not load the Whisper model")?;
    drop(bytes);
    ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
    let mut state = context.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(
        std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 8) as i32,
    );
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_nst(true);
    params.set_progress_callback_safe(progress);
    // whisper-rs 0.16's safe abort helper casts a double-boxed closure to the
    // wrong pointer type. Pass the actual AtomicBool instead. The Arc stays
    // alive until full() returns; native workers only read this atomic flag.
    unsafe {
        params.set_abort_callback(Some(abort_callback));
        params.set_abort_callback_user_data(Arc::as_ptr(&cancel).cast_mut().cast());
    }
    let result = state.full(params, &audio.samples);
    ensure!(!cancel.load(Ordering::Relaxed), "Operation cancelled");
    result.context("Whisper transcription failed")?;
    ensure!(
        fingerprint(source)? == expected_fingerprint,
        "Source file changed during transcription"
    );
    let duration = audio.duration();
    let mut segments = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str_lossy()?.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        let start = (segment.start_timestamp() as f64 / 100.0).clamp(0.0, duration);
        let end = (segment.end_timestamp() as f64 / 100.0).clamp(start, duration);
        segments.push(Segment { start, end, text });
    }
    let language = whisper_rs::get_lang_str(state.full_lang_id_from_state())
        .unwrap_or(language)
        .to_owned();
    let document = Transcript {
        version: 1,
        source: source.to_owned(),
        source_size: expected_fingerprint.0,
        source_modified_ms: expected_fingerprint.1,
        duration,
        model: model.id.into(),
        language,
        segments,
    };
    document.validate()?;
    Ok(document)
}

unsafe extern "C" fn abort_callback(data: *mut std::ffi::c_void) -> bool {
    // SAFETY: installed above with a live Arc<AtomicBool> for the whole call.
    unsafe { &*data.cast::<AtomicBool>() }.load(Ordering::Relaxed)
}
