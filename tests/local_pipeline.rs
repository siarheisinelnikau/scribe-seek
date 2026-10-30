//! Optional checks requiring a real audio device and the downloaded smoke fixtures.
use scribe_seek::{
    audio::{Audio, SAMPLE_RATE},
    playback::Playback,
};
use std::{sync::Arc, time::Duration};

#[test]
#[ignore = "requires a physical default audio output device"]
fn native_player_seeks_plays_and_pauses() {
    // Silent PCM exercises the physical output clock without making test sounds.
    let audio = Audio {
        samples: vec![0.0; SAMPLE_RATE as usize * 4].into(),
        peaks: vec![0.0].into(),
    };
    let mut player = Playback::default();
    player.load(audio);
    player.seek(1.0, true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let position = player.position();
    assert!(
        (1.05..1.8).contains(&position),
        "clock did not advance from seek: {position}"
    );
    player.toggle().unwrap();
    let paused = player.position();
    std::thread::sleep(Duration::from_millis(120));
    assert!((player.position() - paused).abs() < 0.05);
    player.seek(2.0, false).unwrap();
    assert!((player.position() - 2.0).abs() < 0.05);
}

#[test]
#[ignore = "requires .test-data/jfk.wav and downloaded .test-data/data/models/ggml-tiny.bin"]
fn whisper_recognizes_and_cancels_offline() {
    use scribe_seek::{
        audio,
        model::MODELS,
        storage::{Storage, fingerprint},
        transcribe::{self, Request},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".test-data");
    let source = std::fs::canonicalize(root.join("jfk.wav")).unwrap();
    let storage = Storage::at(root.join("data")).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let audio = audio::decode(&source, &cancel, |_| {}).unwrap();
    let identity = fingerprint(&source).unwrap();
    let document = transcribe::transcribe(
        Request {
            audio: &audio,
            source: &source,
            expected_fingerprint: identity,
            model: MODELS[0],
            language: "en",
        },
        &storage,
        cancel.clone(),
        |_| {},
    )
    .unwrap();
    assert!(document.text().to_lowercase().contains("country"));
    assert!(!document.search("AMERICANS").is_empty());
    document.validate().unwrap();

    let long_audio = Audio {
        samples: audio.samples.repeat(20).into(),
        peaks: audio.peaks,
    };
    let flag = cancel.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        flag.store(true, Ordering::Relaxed);
    });
    let result = transcribe::transcribe(
        Request {
            audio: &long_audio,
            source: &source,
            expected_fingerprint: identity,
            model: MODELS[0],
            language: "en",
        },
        &storage,
        cancel,
        |_| {},
    );
    timer.join().unwrap();
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}
