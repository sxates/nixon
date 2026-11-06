use super::*;
use std::time::Duration;
use tauri::Emitter;

fn update(seq: u64, text: &str) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "timestamp": "10:00:00",
        "source": "mixed",
        "sequence_id": seq,
        "chunk_start_time": 0.0,
        "is_partial": false,
        "confidence": 0.9,
        "audio_start_time": seq as f64,
        "audio_end_time": seq as f64 + 1.0,
        "duration": 1.0,
        "channel": "system"
    })
}

fn on_disk(folder: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(folder.join("transcripts.json")).expect("transcripts.json");
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    json["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["text"].as_str().unwrap().to_string())
        .collect()
}

fn noop(_: &TranscriptUpdate) {}

/// The drain-tail bug (specs/0083 W2a task 3). `stop_recording` has already `take()`n the
/// manager out of `RECORDING_MANAGER` (it is never set in this test, which is that state).
/// The transcription task then emits the tail while it drains. Every one of those segments
/// must land in the sink AND in transcripts.json.
#[tokio::test]
async fn segments_emitted_during_the_drain_land_in_the_sink_and_on_disk() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Vec::new()));
    let sink = TranscriptSink::new(store.clone(), Vec::new(), Some(dir.path().to_path_buf()));

    let listener = listen_into_sink(&handle, sink.clone(), noop);
    handle
        .emit("transcript-update", update(1, "before stop"))
        .unwrap();

    // The drain: the worker keeps producing segments after stop was pressed.
    let emitter = handle.clone();
    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        emitter
            .emit("transcript-update", update(2, "drained one"))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        emitter
            .emit("transcript-update", update(3, "drained two"))
            .unwrap();
    });

    let outcome = drain_then_detach(
        &handle,
        Some(task),
        Some(listener),
        Some(&sink),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome, DrainOutcome::Completed);

    let texts: Vec<String> = sink.segments().into_iter().map(|s| s.text).collect();
    assert_eq!(texts, vec!["before stop", "drained one", "drained two"]);
    assert_eq!(on_disk(dir.path()), texts);
    // The saver's own store is the same Arc, so its copy is complete too.
    assert_eq!(store.lock().unwrap().len(), 3);

    // After the drain the listener is gone: nothing from a later session leaks in.
    handle
        .emit("transcript-update", update(9, "after detach"))
        .unwrap();
    assert_eq!(sink.segments().len(), 3);
}

/// The post-drain rewrite: even if an incremental write was missed (here, segments put
/// straight into the store), the drain's end leaves transcripts.json complete, prior
/// sessions first (specs/0037).
#[tokio::test]
async fn drain_rewrites_transcripts_json_with_prior_sessions_first() {
    let app = tauri::test::mock_app();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(Vec::new()));
    let prior = vec![segment_from_update(
        &serde_json::from_value(update(1, "prior session")).unwrap(),
    )];
    let sink = TranscriptSink::new(store.clone(), prior, Some(dir.path().to_path_buf()));
    upsert_segment(
        &store,
        segment_from_update(&serde_json::from_value(update(1, "this session")).unwrap()),
    );

    let outcome = drain_then_detach(
        app.handle(),
        None,
        None,
        Some(&sink),
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(outcome, DrainOutcome::NoTask);
    assert_eq!(on_disk(dir.path()), vec!["prior session", "this session"]);
    // `segments()` is the current session only.
    assert_eq!(sink.segments().len(), 1);
}

#[test]
fn upsert_replaces_by_sequence_id() {
    let store = Mutex::new(Vec::new());
    let first = segment_from_update(&serde_json::from_value(update(4, "draft")).unwrap());
    let second = segment_from_update(&serde_json::from_value(update(4, "final")).unwrap());
    upsert_segment(&store, first);
    upsert_segment(&store, second);
    let segs = store.lock().unwrap();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "final");
    assert_eq!(segs[0].channel.as_deref(), Some("system"));
}
