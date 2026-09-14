//! LLM scheduling for the local providers (specs/0056 W2).
//!
//! One local model serves every LLM call Nixon makes — summaries, background action-item
//! extraction, prep briefs, person roll-ups, Ask AI. Before this module there was no
//! app-level scheduling at all: every job called the provider directly, Ollama queued them
//! FIFO (`OLLAMA_NUM_PARALLEL` defaults to 1), and `reqwest`'s 300 s timeout started at send —
//! so an Ask AI question that arrived while an extraction was generating spent its whole
//! budget *queued* and then timed out. Background jobs also paid each other's queue time
//! against their own timeouts, which is one way "extraction runs for a while, then fails".
//!
//! "Parallel" is not available on one model; **priority** is. The rules:
//!
//! - Calls carry a [`Priority`] via a task-local set at the entry point
//!   ([`with_priority`]). Anything that does not declare one is [`Priority::Background`].
//! - **Background** calls take a global one-permit semaphore (serializing background jobs
//!   among themselves) and wait until no interactive call is in flight before sending.
//! - **Interactive** calls never wait on the semaphore. On Ollama, an interactive arrival
//!   **preempts** a background request in flight: the request future is dropped (Ollama
//!   aborts generation when the client disconnects), the permit is released, and the call is
//!   re-issued from scratch once interactive work drains. Preemption is bounded per call
//!   ([`MAX_PREEMPTIONS`]) so a stream of questions cannot starve background work forever.
//! - The BuiltInAI sidecar cannot cancel a request short of killing the process, so it gets
//!   priority-wait but no preemption.
//! - Cloud providers are remote and parallel: they bypass the gate entirely.
//!
//! Infallible by construction; the only await while holding state is the semaphore permit.

use crate::summary::llm_client::LLMProvider;
use std::future::Future;
use std::sync::OnceLock;
use tokio::sync::{watch, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::info;

/// How many times one background call may be preempted before it proceeds regardless.
pub const MAX_PREEMPTIONS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// A person is waiting on the result right now (Ask AI, person roll-up, "Scan again",
    /// "Regenerate brief").
    Interactive,
    /// Nobody is watching: post-recording summaries, auto extraction, prep passes.
    Background,
}

tokio::task_local! {
    static PRIORITY: Priority;
}

/// Run `fut` with every LLM call inside it scheduled at `priority`. Set once at the entry
/// point (the spawned task or the command body); nested spawns do NOT inherit it and default
/// to [`Priority::Background`], which is the safe direction.
pub async fn with_priority<F: Future>(priority: Priority, fut: F) -> F::Output {
    PRIORITY.scope(priority, fut).await
}

/// The priority of the current task — `Background` when none was declared.
pub fn current_priority() -> Priority {
    PRIORITY.try_with(|p| *p).unwrap_or(Priority::Background)
}

/// Whether the provider is a local model the gate schedules. Cloud providers bypass it.
fn is_local(provider: &LLMProvider) -> bool {
    matches!(provider, LLMProvider::Ollama | LLMProvider::BuiltInAI)
}

/// Whether dropping the request future actually stops generation for this provider.
fn supports_preemption(provider: &LLMProvider) -> bool {
    matches!(provider, LLMProvider::Ollama)
}

pub struct LlmGate {
    background: Semaphore,
    /// Number of interactive calls in flight. `watch` so background callers can await a
    /// transition in either direction without polling.
    interactive: watch::Sender<usize>,
}

impl Default for LlmGate {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmGate {
    pub fn new() -> Self {
        let (interactive, _) = watch::channel(0usize);
        Self {
            background: Semaphore::new(1),
            interactive,
        }
    }

    /// The process-wide gate.
    pub fn global() -> &'static LlmGate {
        static GATE: OnceLock<LlmGate> = OnceLock::new();
        GATE.get_or_init(LlmGate::new)
    }

    pub fn interactive_in_flight(&self) -> usize {
        *self.interactive.borrow()
    }

    /// Mark an interactive call in flight until the guard drops.
    fn enter_interactive(&self) -> InteractiveGuard<'_> {
        self.interactive.send_modify(|n| *n += 1);
        InteractiveGuard { gate: self }
    }

    async fn wait_no_interactive(&self) {
        let mut rx = self.interactive.subscribe();
        // `wait_for` checks the current value first, so this returns immediately when idle.
        // The sender lives as long as `self`, so the receiver cannot observe a closed channel.
        let _ = rx.wait_for(|n| *n == 0).await;
    }

    async fn wait_interactive_requested(&self) {
        let mut rx = self.interactive.subscribe();
        let _ = rx.wait_for(|n| *n > 0).await;
    }

    /// Schedule one provider call. `make_request` must build a *fresh* request future each
    /// time it is called: a preempted background call is re-issued from scratch.
    ///
    /// A background call can wait a long time in here (behind interactive work, or behind
    /// another background job on the one-permit semaphore), so the wait itself races
    /// `cancel`: a cancelled job returns `cancelled_error()` immediately instead of running
    /// once its turn finally comes (review finding on specs/0056 W2).
    pub async fn run<F, Fut, T, E>(
        &self,
        provider: &LLMProvider,
        priority: Priority,
        cancel: Option<&CancellationToken>,
        cancelled_error: impl Fn() -> E,
        mut make_request: F,
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        if !is_local(provider) {
            return make_request().await;
        }

        if priority == Priority::Interactive {
            let _in_flight = self.enter_interactive();
            return make_request().await;
        }

        let mut preemptions: u32 = 0;
        loop {
            tokio::select! {
                _ = self.wait_no_interactive() => {}
                _ = cancelled(cancel) => return Err(cancelled_error()),
            }
            // Semaphore::acquire only errors when the semaphore is closed, which never happens.
            let permit = tokio::select! {
                permit = self.background.acquire() => permit,
                _ = cancelled(cancel) => return Err(cancelled_error()),
            };
            let Ok(_permit) = permit else {
                return make_request().await;
            };
            // An interactive call may have arrived while we waited for the permit — yield
            // before spending any model time.
            if self.interactive_in_flight() > 0 {
                continue;
            }

            if !supports_preemption(provider) || preemptions >= MAX_PREEMPTIONS {
                return make_request().await;
            }

            tokio::select! {
                biased;
                result = make_request() => return result,
                _ = cancelled(cancel) => return Err(cancelled_error()),
                _ = self.wait_interactive_requested() => {
                    preemptions += 1;
                    info!(
                        "llm-gate: background call preempted by interactive work ({}/{}); \
                         re-issuing after it drains",
                        preemptions, MAX_PREEMPTIONS
                    );
                    // `_permit` drops at the end of this iteration, releasing the semaphore
                    // for the loop to re-acquire once interactive work is done.
                }
            }
        }
    }
}

/// Resolves when `cancel` fires; never, when there is no token.
async fn cancelled(cancel: Option<&CancellationToken>) {
    match cancel {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

struct InteractiveGuard<'a> {
    gate: &'a LlmGate,
}

impl Drop for InteractiveGuard<'_> {
    fn drop(&mut self) {
        self.gate
            .interactive
            .send_modify(|n| *n = n.saturating_sub(1));
    }
}

/// Convenience: run one call on the global gate at the current task's priority.
pub async fn run_gated<F, Fut, T, E>(
    provider: &LLMProvider,
    cancel: Option<&CancellationToken>,
    cancelled_error: impl Fn() -> E,
    make_request: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    LlmGate::global()
        .run(
            provider,
            current_priority(),
            cancel,
            cancelled_error,
            make_request,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::{sleep, timeout};

    type BoxedRequest = std::pin::Pin<Box<dyn Future<Output = Result<(), ()>> + Send>>;
    type Releases = Arc<tokio::sync::Mutex<Vec<oneshot::Sender<()>>>>;

    /// A request whose completion the test controls; counts how many times it was built.
    fn controllable(starts: Arc<AtomicUsize>) -> (impl FnMut() -> BoxedRequest, Releases) {
        let releases: Releases = Default::default();
        let releases_for_closure = Arc::clone(&releases);
        let f = move || {
            let starts = Arc::clone(&starts);
            let releases = Arc::clone(&releases_for_closure);
            Box::pin(async move {
                starts.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = oneshot::channel();
                releases.lock().await.push(tx);
                let _ = rx.await;
                Ok(())
            }) as BoxedRequest
        };
        (f, releases)
    }

    async fn release_all(releases: &Releases) {
        for tx in releases.lock().await.drain(..) {
            let _ = tx.send(());
        }
    }

    #[tokio::test]
    async fn cloud_providers_bypass_the_gate_even_while_interactive_work_runs() {
        let gate = Arc::new(LlmGate::new());
        let _busy = gate.enter_interactive();
        let ran = gate
            .run(
                &LLMProvider::OpenAI,
                Priority::Background,
                None,
                || (),
                || async { Ok::<_, ()>(1) },
            )
            .await;
        assert_eq!(ran, Ok(1));
    }

    #[tokio::test]
    async fn background_waits_until_interactive_work_finishes() {
        let gate = Arc::new(LlmGate::new());
        let busy = gate.enter_interactive();
        let starts = Arc::new(AtomicUsize::new(0));
        let (req, releases) = controllable(Arc::clone(&starts));

        let g = Arc::clone(&gate);
        let bg = tokio::spawn(async move {
            g.run(&LLMProvider::Ollama, Priority::Background, None, || (), req)
                .await
        });

        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            0,
            "must not start while interactive"
        );

        drop(busy);
        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "starts once interactive drains"
        );
        release_all(&releases).await;
        assert_eq!(
            timeout(Duration::from_secs(1), bg).await.unwrap().unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn interactive_never_waits_on_the_background_semaphore() {
        let gate = Arc::new(LlmGate::new());
        let bg_starts = Arc::new(AtomicUsize::new(0));
        let (bg_req, bg_releases) = controllable(Arc::clone(&bg_starts));
        let g = Arc::clone(&gate);
        // BuiltInAI: holds the permit, cannot be preempted.
        let bg = tokio::spawn(async move {
            g.run(
                &LLMProvider::BuiltInAI,
                Priority::Background,
                None,
                || (),
                bg_req,
            )
            .await
        });
        sleep(Duration::from_millis(30)).await;
        assert_eq!(bg_starts.load(Ordering::SeqCst), 1);

        let fg = gate
            .run(
                &LLMProvider::BuiltInAI,
                Priority::Interactive,
                None,
                || (),
                || async { Ok::<_, ()>("answer") },
            )
            .await;
        assert_eq!(
            fg,
            Ok("answer"),
            "interactive ran while background held the permit"
        );

        release_all(&bg_releases).await;
        assert_eq!(
            timeout(Duration::from_secs(1), bg).await.unwrap().unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn an_interactive_arrival_preempts_an_in_flight_ollama_background_call() {
        let gate = Arc::new(LlmGate::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let (req, releases) = controllable(Arc::clone(&starts));

        let g = Arc::clone(&gate);
        let bg = tokio::spawn(async move {
            g.run(&LLMProvider::Ollama, Priority::Background, None, || (), req)
                .await
        });
        sleep(Duration::from_millis(30)).await;
        assert_eq!(starts.load(Ordering::SeqCst), 1, "background is generating");

        // Interactive work arrives, is in flight briefly (a real request awaits I/O), finishes.
        let fg = gate
            .run(
                &LLMProvider::Ollama,
                Priority::Interactive,
                None,
                || (),
                || async {
                    sleep(Duration::from_millis(20)).await;
                    Ok::<_, ()>(())
                },
            )
            .await;
        assert_eq!(fg, Ok(()));

        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            2,
            "the background request was dropped and re-issued from scratch"
        );
        release_all(&releases).await;
        assert_eq!(
            timeout(Duration::from_secs(1), bg).await.unwrap().unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn preemption_is_bounded_so_background_work_is_never_starved() {
        let gate = Arc::new(LlmGate::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let (req, releases) = controllable(Arc::clone(&starts));

        let g = Arc::clone(&gate);
        let bg = tokio::spawn(async move {
            g.run(&LLMProvider::Ollama, Priority::Background, None, || (), req)
                .await
        });
        sleep(Duration::from_millis(30)).await;

        for _ in 0..MAX_PREEMPTIONS + 2 {
            let _ = gate
                .run(
                    &LLMProvider::Ollama,
                    Priority::Interactive,
                    None,
                    || (),
                    || async {
                        sleep(Duration::from_millis(20)).await;
                        Ok::<_, ()>(())
                    },
                )
                .await;
            sleep(Duration::from_millis(30)).await;
        }
        // 1 original + MAX_PREEMPTIONS re-issues; the extra interactive calls did not
        // preempt again.
        assert_eq!(
            starts.load(Ordering::SeqCst) as u32,
            1 + MAX_PREEMPTIONS,
            "preemptions stop at the bound"
        );
        release_all(&releases).await;
        assert_eq!(
            timeout(Duration::from_secs(1), bg).await.unwrap().unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn background_calls_serialize_among_themselves() {
        let gate = Arc::new(LlmGate::new());
        let starts_a = Arc::new(AtomicUsize::new(0));
        let starts_b = Arc::new(AtomicUsize::new(0));
        let (req_a, rel_a) = controllable(Arc::clone(&starts_a));
        let (req_b, rel_b) = controllable(Arc::clone(&starts_b));

        let ga = Arc::clone(&gate);
        let a = tokio::spawn(async move {
            ga.run(
                &LLMProvider::Ollama,
                Priority::Background,
                None,
                || (),
                req_a,
            )
            .await
        });
        sleep(Duration::from_millis(30)).await;
        let gb = Arc::clone(&gate);
        let b = tokio::spawn(async move {
            gb.run(
                &LLMProvider::Ollama,
                Priority::Background,
                None,
                || (),
                req_b,
            )
            .await
        });
        sleep(Duration::from_millis(30)).await;
        assert_eq!(starts_a.load(Ordering::SeqCst), 1);
        assert_eq!(starts_b.load(Ordering::SeqCst), 0, "B waits for A's permit");

        release_all(&rel_a).await;
        assert_eq!(
            timeout(Duration::from_secs(1), a).await.unwrap().unwrap(),
            Ok(())
        );
        sleep(Duration::from_millis(30)).await;
        assert_eq!(
            starts_b.load(Ordering::SeqCst),
            1,
            "B starts once A is done"
        );
        release_all(&rel_b).await;
        assert_eq!(
            timeout(Duration::from_secs(1), b).await.unwrap().unwrap(),
            Ok(())
        );
    }

    /// Review finding (specs/0056 W2): a background call cancelled while it is still QUEUED
    /// must return immediately, not run once its turn comes.
    #[tokio::test]
    async fn a_queued_background_call_observes_cancellation_while_waiting() {
        let gate = Arc::new(LlmGate::new());
        let _busy = gate.enter_interactive(); // keeps every background call queued
        let token = CancellationToken::new();
        let starts = Arc::new(AtomicUsize::new(0));
        let (req, _releases) = controllable(Arc::clone(&starts));

        let g = Arc::clone(&gate);
        let t = token.clone();
        let bg = tokio::spawn(async move {
            g.run(
                &LLMProvider::Ollama,
                Priority::Background,
                Some(&t),
                || (),
                req,
            )
            .await
        });
        sleep(Duration::from_millis(30)).await;
        token.cancel();
        let out = timeout(Duration::from_secs(1), bg).await.unwrap().unwrap();
        assert_eq!(out, Err(()), "the cancelled error, not a model result");
        assert_eq!(starts.load(Ordering::SeqCst), 0, "never reached the model");
    }

    #[tokio::test]
    async fn priority_defaults_to_background_and_is_scoped() {
        assert_eq!(current_priority(), Priority::Background);
        let inner = with_priority(Priority::Interactive, async { current_priority() }).await;
        assert_eq!(inner, Priority::Interactive);
        assert_eq!(current_priority(), Priority::Background);
    }
}
