//! Process-wide STT inference lock (spec 0045 WS1b).
//!
//! Live transcription and batch retranscription share the same global whisper/Parakeet
//! engine context (`WHISPER_ENGINE` / `PARAKEET_ENGINE`). Once deferred processing is allowed
//! to run concurrently with a live recording, two decode calls could hit the same context at
//! once — which whisper.cpp/Parakeet is not safe against. This mutex serializes the actual
//! decode so they take turns (the batch segment waits behind each live frame). Memory-cheap
//! alternative to a second resident engine; the small added live-frame latency is the
//! accepted contention trade-off (owner decision, spec 0045).

use tokio::sync::Mutex;

static INFERENCE_LOCK: Mutex<()> = Mutex::const_new(());

/// Acquire exclusive access to STT decode. Held for the duration of one inference call.
pub async fn acquire_inference_lock() -> tokio::sync::MutexGuard<'static, ()> {
    INFERENCE_LOCK.lock().await
}

#[cfg(test)]
mod tests {
    use super::acquire_inference_lock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn only_one_holder_at_a_time() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = concurrent.clone();
            let m = max.clone();
            handles.push(tokio::spawn(async move {
                let _g = acquire_inference_lock().await;
                let now = c.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                c.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            max.load(Ordering::SeqCst),
            1,
            "inference lock must serialize decode"
        );
    }
}
