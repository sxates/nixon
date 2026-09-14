# Background LLM Activity + Served-Context Detection — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound every Ollama generation with `max_tokens`, stop the pre-call-prep loop from retrying a broken brief forever, budget context from what the server actually serves, and surface background LLM work in the sidebar.

**Architecture:** Four independent backend fixes plus one new observability surface. A new `llm_activity` module holds an in-memory registry of LLM *tasks* (not individual calls — `generate_summary` is the single choke point for every provider call but is too low-level to name a unit of work). Jobs declare a task via an RAII handle; every state transition emits a Tauri event that a React context provider renders as a sidebar row.

**Tech Stack:** Rust (Tauri 2, sqlx/SQLite, tokio, reqwest, anyhow), TypeScript (Next.js 14, React 18), Vitest, `cargo test`.

**Spec:** `specs/0052-llm-activity-and-context-detection.md`

## Global Constraints

- Work on branch `feat/0052-llm-activity-and-context-detection` (already created off `main`).
- New Tauri commands register in `frontend/src-tauri/src/registry.rs`, **never** `lib.rs` (specs/0042).
- No production source file may exceed **800 lines** (`scripts/check-file-size.sh`); allowlisted files may only shrink. Prefer new modules over growing existing ones.
- Rust errors use `anyhow::Result`; frontend↔Rust is the Tauri command/event pattern.
- Registry operations must **never** propagate failure into generation — a broken indicator must not break summarization (same contract as specs/0034 extraction).
- Rust→TS payloads serialize `camelCase` via `#[serde(rename_all = "camelCase")]`.
- All Rust commands run from `frontend/src-tauri` after `source ~/.cargo/env`, with `--features metal`.
- Do **not** commit the pre-existing uncommitted `Cargo.toml` change (`[profile.dev] incremental = false`) — it is unrelated local infra work.
- Never add a recording guard to the prep pass. Background generation running during a meeting is an explicit owner decision; the indicator is the mitigation.

---

## File Structure

**Create:**
- `frontend/src-tauri/migrations/20260818000000_add_meeting_briefs_failure_count.sql` — schema
- `frontend/src-tauri/src/ollama/served_context.rs` — `/api/ps` probe + TTL cache
- `frontend/src-tauri/src/llm_activity/mod.rs` — re-exports
- `frontend/src-tauri/src/llm_activity/registry.rs` — registry, RAII handle, types
- `frontend/src-tauri/src/llm_activity/commands.rs` — IPC
- `frontend/src/contexts/LlmActivityProvider.tsx` — event listener + snapshot
- `frontend/src/components/LlmActivity/LlmActivityRow.tsx` — sidebar row
- `frontend/src/components/LlmActivity/LlmActivityPopover.tsx` — history popover
- Tests alongside each.

**Modify:**
- `src/ollama/mod.rs`, `src/lib.rs` (module decl + `.manage`), `src/registry.rs` (commands)
- `src/summary/service.rs:69-95` (clamp), `src/summary/llm_client.rs` (default cap)
- `src/summary/processor.rs`, `src/aggregation/prep_jobs.rs`, `src/action_items/extractor.rs` (call-site caps + instrumentation)
- `src/database/repositories/meeting_brief.rs` (failure_count accessors)
- `frontend/src/app/layout.tsx`, `frontend/src/components/Sidebar/index.tsx` (mount)

**Note on state placement:** the spec says the registry lives "in `AppState`". `AppState` (`src/state.rs`) currently holds only `db_manager`, and the codebase already uses standalone managed state (`.manage(audio::init_system_audio_state())`, `lib.rs:150`). This plan uses a separate `.manage()`d `LlmActivityState` instead — same lifetime, cleaner boundary, and it avoids touching `AppState`'s constructor.

---

### Task 1: `meeting_briefs.failure_count` schema + repository

**Files:**
- Create: `frontend/src-tauri/migrations/20260818000000_add_meeting_briefs_failure_count.sql`
- Modify: `frontend/src-tauri/src/database/repositories/meeting_brief.rs`
- Test: `frontend/src-tauri/src/database/repositories/meeting_brief.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing (first task)
- Produces:
  - `MeetingBrief.failure_count: i64` (new struct field)
  - `MeetingBriefsRepository::record_failure(pool: &SqlitePool, meeting_id: &str) -> Result<i64, SqlxError>` — increments and returns the new count
  - `MeetingBriefsRepository::reset_failures(pool: &SqlitePool, meeting_id: &str) -> Result<(), SqlxError>`

- [ ] **Step 1: Write the migration**

Create `frontend/src-tauri/migrations/20260818000000_add_meeting_briefs_failure_count.sql`:

```sql
-- specs/0052: bound the pre-call-prep retry loop.
--
-- prep_jobs.rs recorded status='failed' with no fingerprint and no counter, deliberately so
-- the next pass retries. There was no give-up, so a permanently-failing brief re-ran every
-- 30 minutes forever, each attempt burning up to 3 x 300s of GPU. This counter lets the
-- background pass stop after 3 consecutive failures until the inputs change.
--
-- Reset to 0 on success, or whenever source_fingerprint changes (new input gets fresh
-- attempts). A manual retry from the activity indicator also clears it.
ALTER TABLE meeting_briefs ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `src/database/repositories/meeting_brief.rs` (create the block if absent, following the pattern in sibling repository files):

```rust
#[cfg(test)]
mod failure_count_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool with migrations applied, and one meeting row to satisfy the FK.
    async fn pool_with_meeting(meeting_id: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(meeting_id)
            .bind("Weekly 1:1")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("seed meeting");
        pool
    }

    #[tokio::test]
    async fn record_failure_increments_and_reset_zeroes() {
        let pool = pool_with_meeting("m1").await;
        MeetingBriefsRepository::upsert_status(&pool, "m1", "failed", None)
            .await
            .unwrap();

        assert_eq!(
            MeetingBriefsRepository::record_failure(&pool, "m1").await.unwrap(),
            1
        );
        assert_eq!(
            MeetingBriefsRepository::record_failure(&pool, "m1").await.unwrap(),
            2
        );

        let brief = MeetingBriefsRepository::get(&pool, "m1").await.unwrap().unwrap();
        assert_eq!(brief.failure_count, 2);

        MeetingBriefsRepository::reset_failures(&pool, "m1").await.unwrap();
        let brief = MeetingBriefsRepository::get(&pool, "m1").await.unwrap().unwrap();
        assert_eq!(brief.failure_count, 0);
    }

    #[tokio::test]
    async fn upsert_ready_resets_failure_count() {
        let pool = pool_with_meeting("m2").await;
        MeetingBriefsRepository::upsert_status(&pool, "m2", "failed", None).await.unwrap();
        MeetingBriefsRepository::record_failure(&pool, "m2").await.unwrap();

        MeetingBriefsRepository::upsert_ready(
            &pool, "m2", "# brief", "[]", "fp-1", "ollama", "gemma4:26b",
        )
        .await
        .unwrap();

        let brief = MeetingBriefsRepository::get(&pool, "m2").await.unwrap().unwrap();
        assert_eq!(brief.failure_count, 0, "a successful generation clears the counter");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal failure_count_tests 2>&1 | tail -20
```

Expected: FAIL — `no function or associated item named 'record_failure'`, and `no field 'failure_count' on type 'MeetingBrief'`.

- [ ] **Step 4: Add the struct field and the SELECT column**

In `src/database/repositories/meeting_brief.rs`, add to `MeetingBrief` (after `updated_at`, line 27):

```rust
    /// Consecutive failed generation passes (specs/0052). The background pass stops
    /// retrying at 3; reset to 0 on success or when `source_fingerprint` changes.
    pub failure_count: i64,
```

Then add `failure_count` to the column list in `get` (line 39-41):

```rust
        sqlx::query_as::<_, MeetingBrief>(
            "SELECT meeting_id, status, brief_markdown, sources_json, source_fingerprint, \
                    model_provider, model_name, generated_at, created_at, updated_at, \
                    failure_count \
             FROM meeting_briefs WHERE meeting_id = ?",
        )
```

- [ ] **Step 5: Implement the two accessors**

Add to `impl MeetingBriefsRepository`:

```rust
    /// Increment the consecutive-failure counter and return its new value (specs/0052).
    /// Assumes the row exists — callers write `status = 'failed'` immediately before.
    pub async fn record_failure(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<i64, SqlxError> {
        let count: i64 = sqlx::query_scalar(
            "UPDATE meeting_briefs SET failure_count = failure_count + 1, updated_at = ? \
             WHERE meeting_id = ? RETURNING failure_count",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(meeting_id)
        .fetch_one(pool)
        .await?;
        Ok(count)
    }

    /// Clear the consecutive-failure counter — on success, on a fingerprint change, or on
    /// an explicit user retry (specs/0052).
    pub async fn reset_failures(pool: &SqlitePool, meeting_id: &str) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE meeting_briefs SET failure_count = 0, updated_at = ? WHERE meeting_id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 6: Make `upsert_ready` reset the counter**

In `upsert_ready` (line 49), add `failure_count = 0` to the `DO UPDATE SET` clause. A successful generation always clears the count.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo test --features metal failure_count_tests 2>&1 | tail -20
```

Expected: PASS (2 tests).

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/migrations/20260818000000_add_meeting_briefs_failure_count.sql \
        frontend/src-tauri/src/database/repositories/meeting_brief.rs
git commit -m "feat(0052): add meeting_briefs.failure_count with repository accessors"
```

---

### Task 2: Served-context probe via `/api/ps`

**Files:**
- Create: `frontend/src-tauri/src/ollama/served_context.rs`
- Modify: `frontend/src-tauri/src/ollama/mod.rs`
- Test: inline `#[cfg(test)]` in `served_context.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub async fn served_context(model_name: &str, endpoint: Option<&str>) -> Option<usize>` — cached, returns `None` when unknown
  - `pub async fn refresh_served_context(model_name: &str, endpoint: Option<&str>)` — fire-and-forget re-probe, called after a successful LLM call
  - `pub(crate) fn parse_served_context(body: &str, model_name: &str) -> Option<usize>` — pure parser, unit-testable

**Why a new file:** `metadata.rs` is already ~370 lines and this is a distinct concern on a different refresh schedule (metadata has a 5-minute TTL keyed by model; served context is refreshed opportunistically after real calls).

- [ ] **Step 1: Write the failing test**

Create `frontend/src-tauri/src/ollama/served_context.rs` with only the test module and a stub:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PS_BODY: &str = r#"{
      "models": [
        {"name":"gemma4:26b","model":"gemma4:26b","size":22800000000,"context_length":65536}
      ]
    }"#;

    #[test]
    fn parses_context_length_for_the_named_model() {
        assert_eq!(parse_served_context(PS_BODY, "gemma4:26b"), Some(65536));
    }

    #[test]
    fn returns_none_when_the_model_is_not_loaded() {
        assert_eq!(parse_served_context(r#"{"models":[]}"#, "gemma4:26b"), None);
    }

    #[test]
    fn returns_none_when_a_different_model_is_loaded() {
        assert_eq!(parse_served_context(PS_BODY, "qwen3.8:27b"), None);
    }

    /// Older/newer Ollama builds may omit the field entirely. Degrade, never guess.
    #[test]
    fn returns_none_when_context_length_is_absent() {
        let body = r#"{"models":[{"name":"gemma4:26b","model":"gemma4:26b"}]}"#;
        assert_eq!(parse_served_context(body, "gemma4:26b"), None);
    }

    /// `ollama ps` reports the tag-qualified name; a bare name should still match.
    #[test]
    fn matches_on_the_model_field_too() {
        let body = r#"{"models":[{"name":"x","model":"gemma4:26b","context_length":32768}]}"#;
        assert_eq!(parse_served_context(body, "gemma4:26b"), Some(32768));
    }

    #[test]
    fn returns_none_on_malformed_json() {
        assert_eq!(parse_served_context("not json", "gemma4:26b"), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal served_context 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'parse_served_context'` (and the module is not yet declared).

- [ ] **Step 3: Implement the parser and cache**

Prepend to `served_context.rs` (above the test module):

```rust
//! Detect the context Ollama is ACTUALLY serving (specs/0052).
//!
//! `/api/show` reports `<family>.context_length` — the model's ARCHITECTURAL maximum
//! (262,144 for gemma4). What the server allocates is `OLLAMA_CONTEXT_LENGTH`, or Ollama's
//! VRAM-based default. Budgeting from the architectural max meant Vinyl believed it had 4x
//! the room it had, never reached the chunking path, and let long meetings be silently
//! context-shifted.
//!
//! `/api/ps` reports the loaded runner's real allocated `context_length`, but only while a
//! model is loaded. So we cache the last known value and refresh it opportunistically after
//! a successful LLM call — when the model is guaranteed loaded — rather than forcing an
//! ~11s cold load just to probe.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Last known served context per `"{endpoint}|{model}"`. No TTL: a stale reading is far
/// better than none (the fallback is a very conservative floor), and it is refreshed after
/// every successful call.
static SERVED_CACHE: Lazy<Arc<RwLock<HashMap<String, usize>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

const DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn cache_key(model_name: &str, endpoint: Option<&str>) -> String {
    format!("{}|{}", endpoint.unwrap_or(DEFAULT_ENDPOINT), model_name)
}

/// Pure parser for an `/api/ps` body. `None` whenever the model isn't loaded, the field is
/// absent, or the body doesn't parse — never a guess.
pub(crate) fn parse_served_context(body: &str, model_name: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("models")?.as_array()?.iter().find_map(|m| {
        let matches = m.get("name").and_then(|n| n.as_str()) == Some(model_name)
            || m.get("model").and_then(|n| n.as_str()) == Some(model_name);
        if !matches {
            return None;
        }
        m.get("context_length")
            .and_then(|c| c.as_u64())
            .map(|c| c as usize)
    })
}

async fn probe(model_name: &str, endpoint: Option<&str>) -> Option<usize> {
    let base = endpoint.unwrap_or(DEFAULT_ENDPOINT);
    let body = crate::config::shared_http_client()
        .get(format!("{base}/api/ps"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    parse_served_context(&body, model_name)
}

/// Served context for a model: a live `/api/ps` reading when the model is loaded, else the
/// last cached reading, else `None` (caller applies its own floor).
pub async fn served_context(model_name: &str, endpoint: Option<&str>) -> Option<usize> {
    let key = cache_key(model_name, endpoint);
    if let Some(ctx) = probe(model_name, endpoint).await {
        SERVED_CACHE.write().await.insert(key, ctx);
        return Some(ctx);
    }
    SERVED_CACHE.read().await.get(&key).copied()
}

/// Re-probe and update the cache. Call after a SUCCESSFUL LLM call, when the model is
/// guaranteed loaded. Best-effort: never surfaces an error to the caller.
pub async fn refresh_served_context(model_name: &str, endpoint: Option<&str>) {
    if let Some(ctx) = probe(model_name, endpoint).await {
        SERVED_CACHE
            .write()
            .await
            .insert(cache_key(model_name, endpoint), ctx);
    }
}
```

- [ ] **Step 4: Declare the module**

In `src/ollama/mod.rs`, add alongside the existing `pub mod metadata;`:

```rust
pub mod served_context;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test --features metal served_context 2>&1 | tail -20
```

Expected: PASS (6 tests).

- [ ] **Step 6: Verify against the real server (RISK CHECK)**

The spec's top risk is that the `/api/ps` field shape differs on Ollama 0.32.12 (confirmed only on 0.24.0). Run this **on the machine that runs Vinyl**, with a model loaded:

```bash
curl -s localhost:11434/api/ps | python3 -m json.tool
```

Confirm a `context_length` field exists on the model entry and that its value tracks `OLLAMA_CONTEXT_LENGTH` rather than the model's architectural max. If the field is named differently, fix `parse_served_context` and add a test case for the real shape before continuing. **If it reports per-slot context under `OLLAMA_NUM_PARALLEL > 1`, note it and stop — the clamp in Task 3 would be wrong.**

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/ollama/served_context.rs frontend/src-tauri/src/ollama/mod.rs
git commit -m "feat(0052): probe Ollama /api/ps for the actually-served context size"
```

---

### Task 3: Clamp the context budget to the served size

**Files:**
- Modify: `frontend/src-tauri/src/summary/service.rs:69-95`
- Test: inline `#[cfg(test)]` in `src/summary/service.rs`

**Interfaces:**
- Consumes: `ollama::served_context::served_context` (Task 2)
- Produces: `pub(crate) fn clamp_context_budget(arch_max: usize, served: Option<usize>) -> usize` — pure, unit-testable

- [ ] **Step 1: Write the failing test**

Add to `src/summary/service.rs`:

```rust
#[cfg(test)]
mod context_clamp_tests {
    use super::*;

    #[test]
    fn clamps_to_the_served_context_when_smaller() {
        // The real bug: gemma4 reports a 262,144 architectural max while the server
        // serves 65,536. Budgeting from the arch max disabled chunking entirely.
        assert_eq!(clamp_context_budget(262_144, Some(65_536)), 65_236);
    }

    #[test]
    fn uses_the_architectural_max_when_it_is_smaller() {
        assert_eq!(clamp_context_budget(8_192, Some(65_536)), 7_892);
    }

    #[test]
    fn falls_back_to_the_conservative_floor_when_served_is_unknown() {
        assert_eq!(clamp_context_budget(262_144, None), CONSERVATIVE_CONTEXT_FLOOR);
    }

    #[test]
    fn never_underflows_on_a_tiny_architectural_max() {
        assert_eq!(clamp_context_budget(100, Some(100)), 1);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal context_clamp_tests 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'clamp_context_budget'`.

- [ ] **Step 3: Implement the clamp**

Add near the top of `src/summary/service.rs`, beside the existing constants:

```rust
/// Budget floor when the served context is unknown (model idle and nothing cached). Small
/// enough to be safe on any endpoint; the chunking path handles the rest.
pub(crate) const CONSERVATIVE_CONTEXT_FLOOR: usize = 8_192;

/// Tokens reserved for prompt scaffolding — unchanged from the original implementation.
const CONTEXT_RESERVE: usize = 300;

/// The usable budget: never more than the server actually serves (specs/0052).
///
/// `/api/show` reports the model's ARCHITECTURAL max; `/api/ps` reports what is allocated.
/// Taking the architectural max meant Vinyl budgeted ~261,844 against a 65,536 server, so
/// summarization never chunked and long meetings were silently context-shifted.
pub(crate) fn clamp_context_budget(arch_max: usize, served: Option<usize>) -> usize {
    match served {
        Some(served) => arch_max.min(served).saturating_sub(CONTEXT_RESERVE).max(1),
        None => CONSERVATIVE_CONTEXT_FLOOR,
    }
}
```

- [ ] **Step 4: Wire it into `resolve_context_budget`**

Replace the `Ok(metadata)` arm of the Ollama branch (`service.rs:79-87`) with:

```rust
            Ok(metadata) => {
                let served =
                    crate::ollama::served_context::served_context(model_name, ollama_endpoint)
                        .await;
                let budget = clamp_context_budget(metadata.context_size, served);
                info!(
                    "✓ Context budget for {}: {} tokens (architectural {}, served {:?})",
                    model_name, budget, metadata.context_size, served
                );
                budget
            }
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test --features metal context_clamp_tests 2>&1 | tail -20
cargo check --features metal 2>&1 | tail -5
```

Expected: PASS (4 tests), clean check.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/summary/service.rs
git commit -m "feat(0052): clamp the Ollama context budget to the served context size"
```

---

### Task 4: `max_tokens` is never unbounded for Ollama

**Files:**
- Modify: `frontend/src-tauri/src/summary/llm_client.rs`
- Modify: `frontend/src-tauri/src/aggregation/prep_jobs.rs` (prep cap)
- Modify: `frontend/src-tauri/src/summary/processor.rs` (chunk-map cap)
- Test: inline `#[cfg(test)]` in `src/summary/llm_client.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub const DEFAULT_OLLAMA_MAX_TOKENS: u32 = 4096;`
  - `pub const PREP_BRIEF_MAX_TOKENS: u32 = 2000;`
  - `pub const CHUNK_MAP_MAX_TOKENS: u32 = 1500;`

**Note:** action-item extraction (`extractor.rs:490`) deliberately keeps the *default* rather than a tight cap — its existing comment reads "an explicit cap risks truncated JSON", and truncated JSON is a worse failure than a slow call. It moves from unbounded to 4096 automatically via the default; **do not** pass an explicit value there.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/summary/llm_client.rs` (beside `chat_request_omits_none_optionals_but_keeps_temperature`, line 419):

```rust
    /// The regression that started specs/0052: provider_config.rs hardcodes `None` for
    /// Ollama's max_tokens and ChatRequest skips the field when None, so every Ollama
    /// request shipped with NO output cap and ran to the 300s timeout.
    #[test]
    fn ollama_requests_always_carry_a_max_tokens_cap() {
        let body = build_chat_request_body(&LLMProvider::Ollama, "gemma4:26b", "sys", "usr", None, None, None);
        assert_eq!(
            body["max_tokens"], DEFAULT_OLLAMA_MAX_TOKENS,
            "an uncapped Ollama request can generate until the request times out"
        );
    }

    #[test]
    fn an_explicit_cap_wins_over_the_ollama_default() {
        let body = build_chat_request_body(
            &LLMProvider::Ollama, "gemma4:26b", "sys", "usr", Some(2000), None, None,
        );
        assert_eq!(body["max_tokens"], 2000);
    }

    /// Non-Ollama OpenAI-compatible providers keep provider-default behavior.
    #[test]
    fn non_ollama_openai_providers_still_omit_max_tokens_when_unset() {
        let body = build_chat_request_body(&LLMProvider::OpenAI, "gpt-4o", "sys", "usr", None, None, None);
        assert!(body.get("max_tokens").is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal --lib llm_client 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'build_chat_request_body'`.

- [ ] **Step 3: Extract the request-body builder**

The body is currently built inline in `generate_summary` (`llm_client.rs:252-284`). Extract it so it is testable without a live HTTP call. Add:

```rust
/// Fallback output cap for Ollama. `provider_config.rs` has no path to set one, so without
/// this every Ollama request was unbounded and ran to REQUEST_TIMEOUT_DURATION (300s).
/// 4096 at ~60 tok/s is ~70s — generous enough not to truncate a real report, tight enough
/// that a degenerate loop is a blip rather than a five-minute GPU burn.
pub const DEFAULT_OLLAMA_MAX_TOKENS: u32 = 4096;

/// Prep briefs synthesize 2 prior summaries; p90 of healthy runs was ~1,800 tokens.
pub const PREP_BRIEF_MAX_TOKENS: u32 = 2000;

/// Per-chunk map summaries are short by construction.
pub const CHUNK_MAP_MAX_TOKENS: u32 = 1500;

/// Build the provider request body. Split out of `generate_summary` so the token-cap and
/// temperature policy is unit-testable without a live endpoint.
pub(crate) fn build_chat_request_body(
    provider: &LLMProvider,
    model_name: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
) -> serde_json::Value {
    let effective_temperature = Some(temperature.unwrap_or(DEFAULT_SUMMARY_TEMPERATURE));

    // Ollama has no settings path for max_tokens, so apply the default here rather than
    // leaving the field absent (which means "unbounded").
    let effective_max_tokens = match provider {
        LLMProvider::Ollama => Some(max_tokens.unwrap_or(DEFAULT_OLLAMA_MAX_TOKENS)),
        _ => max_tokens,
    };

    if provider != &LLMProvider::Claude {
        serde_json::json!(ChatRequest {
            model: model_name.to_string(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: system_prompt.to_string() },
                ChatMessage { role: "user".to_string(), content: user_prompt.to_string() },
            ],
            max_tokens: effective_max_tokens,
            temperature: effective_temperature,
            top_p,
        })
    } else {
        serde_json::json!(ClaudeRequest {
            system: system_prompt.to_string(),
            model: model_name.to_string(),
            max_tokens: max_tokens.unwrap_or(DEFAULT_CLAUDE_MAX_TOKENS),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            }],
            temperature: effective_temperature,
        })
    }
}
```

Then replace the inline block in `generate_summary` (the `let request_body = if provider != &LLMProvider::Claude { ... }` expression at lines 252-284) with:

```rust
    let request_body = build_chat_request_body(
        provider, model_name, system_prompt, user_prompt, max_tokens, temperature, top_p,
    );
```

Delete the now-unused `effective_temperature` binding at line 249.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --features metal --lib llm_client 2>&1 | tail -20
```

Expected: PASS — the three new tests plus the pre-existing ones.

- [ ] **Step 5: Apply the per-call-site caps**

In `src/aggregation/prep_jobs.rs`, inside the `llm` closure (the `cfg.custom_openai_max_tokens` argument, ~line 236), replace it with:

```rust
                cfg.custom_openai_max_tokens
                    .or(Some(crate::summary::llm_client::PREP_BRIEF_MAX_TOKENS)),
```

In `src/summary/processor.rs`, at the chunk-map `generate_summary_with_retry` call (~line 899), replace the `max_tokens` argument with:

```rust
                    max_tokens.or(Some(crate::summary::llm_client::CHUNK_MAP_MAX_TOKENS)),
```

A user-configured CustomOpenAI value still wins in both cases.

- [ ] **Step 6: Verify the build and full suite**

```bash
cargo clippy --features metal 2>&1 | tail -10
cargo test --features metal 2>&1 | tail -20
```

Expected: no clippy warnings, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/summary/llm_client.rs \
        frontend/src-tauri/src/aggregation/prep_jobs.rs \
        frontend/src-tauri/src/summary/processor.rs
git commit -m "fix(0052): never send an uncapped Ollama request; add per-call-site max_tokens"
```

---

### Task 5: Prep-brief give-up after 3 consecutive failures

**Files:**
- Modify: `frontend/src-tauri/src/aggregation/prep_jobs.rs`
- Test: inline `#[cfg(test)]` in `src/aggregation/prep_jobs.rs`

**Interfaces:**
- Consumes: `MeetingBriefsRepository::{record_failure, reset_failures}`, `MeetingBrief.failure_count` (Task 1)
- Produces: `pub(crate) const MAX_BRIEF_FAILURES: i64 = 3;` and `pub(crate) fn should_skip_brief(existing: Option<&MeetingBrief>, current_fingerprint: &str) -> bool`

- [ ] **Step 1: Write the failing test**

Add to `src/aggregation/prep_jobs.rs`:

```rust
#[cfg(test)]
mod give_up_tests {
    use super::*;
    use crate::database::repositories::meeting_brief::MeetingBrief;

    fn brief(status: &str, fingerprint: Option<&str>, failure_count: i64) -> MeetingBrief {
        MeetingBrief {
            meeting_id: "m1".into(),
            status: status.into(),
            brief_markdown: None,
            sources_json: None,
            source_fingerprint: fingerprint.map(|s| s.to_string()),
            model_provider: None,
            model_name: None,
            generated_at: None,
            created_at: "2026-08-18T00:00:00Z".into(),
            updated_at: "2026-08-18T00:00:00Z".into(),
            failure_count,
        }
    }

    #[test]
    fn keeps_retrying_below_the_cap() {
        assert!(!should_skip_brief(Some(&brief("failed", Some("fp-1"), 2)), "fp-1"));
    }

    #[test]
    fn gives_up_at_the_cap() {
        assert!(should_skip_brief(Some(&brief("failed", Some("fp-1"), 3)), "fp-1"));
    }

    /// New input always earns a fresh set of attempts.
    #[test]
    fn a_fingerprint_change_resumes_a_given_up_brief() {
        assert!(!should_skip_brief(Some(&brief("failed", Some("fp-1"), 9)), "fp-2"));
    }

    #[test]
    fn a_brief_with_no_row_is_never_skipped() {
        assert!(!should_skip_brief(None, "fp-1"));
    }

    /// A 'failed' row with no fingerprint recorded (the pre-0052 shape) must not be
    /// treated as matching the current fingerprint.
    #[test]
    fn a_null_fingerprint_never_matches() {
        assert!(!should_skip_brief(Some(&brief("failed", None, 9)), "fp-1"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal give_up_tests 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'should_skip_brief'`.

- [ ] **Step 3: Implement the predicate**

Add to `src/aggregation/prep_jobs.rs` beside the other constants:

```rust
/// Consecutive failed passes before the background generator stops retrying a brief
/// (specs/0052). Bounds a permanently-broken brief at ~45 min of GPU instead of forever,
/// while still self-healing from a transient failure (Ollama restarting, model swapping).
pub(crate) const MAX_BRIEF_FAILURES: i64 = 3;

/// Whether the background pass should skip this brief entirely.
///
/// Only skips a brief that has failed `MAX_BRIEF_FAILURES` times against the SAME input:
/// a fingerprint change means new input, which always earns a fresh set of attempts.
pub(crate) fn should_skip_brief(
    existing: Option<&MeetingBrief>,
    current_fingerprint: &str,
) -> bool {
    let Some(existing) = existing else {
        return false;
    };
    existing.failure_count >= MAX_BRIEF_FAILURES
        && existing.source_fingerprint.as_deref() == Some(current_fingerprint)
}
```

Add the import: `use crate::database::repositories::meeting_brief::MeetingBrief;`

- [ ] **Step 4: Wire it into `generate_brief_for_target`**

In `generate_brief_for_target`, the existing up-to-date check (lines 176-183) already loads the row and computes `fingerprint`. Replace that block with one that also handles skip and reset:

```rust
    let fingerprint = source_fingerprint(pool, &prior).await;
    let existing = MeetingBriefsRepository::get(pool, target_meeting_id).await?;

    if !force {
        if let Some(existing) = existing.as_ref() {
            if existing.status == "ready"
                && existing.source_fingerprint.as_deref() == Some(fingerprint.as_str())
            {
                return Ok(()); // up to date
            }
        }
        // specs/0052: stop burning GPU on a brief that has failed repeatedly against
        // input that has not changed. Resumes on a fingerprint change or a manual retry.
        if should_skip_brief(existing.as_ref(), &fingerprint) {
            info!(
                "prep: skipping {} — {} consecutive failures against unchanged input",
                target_meeting_id, MAX_BRIEF_FAILURES
            );
            return Ok(());
        }
    }

    // New input => fresh attempts (the reset must happen BEFORE the skip check can matter
    // on the next pass, and before this pass records a failure of its own).
    if existing
        .as_ref()
        .is_some_and(|e| e.source_fingerprint.as_deref() != Some(fingerprint.as_str()))
    {
        MeetingBriefsRepository::reset_failures(pool, target_meeting_id).await?;
    }
```

- [ ] **Step 5: Record failures in the `Err` arm**

Replace the `Err(e)` arm at the end of `generate_brief_for_target` (line 262):

```rust
        Err(e) => {
            // No fingerprint on 'failed' → retried on the next pass (self-heals transient
            // errors), but bounded now: after MAX_BRIEF_FAILURES consecutive failures
            // against unchanged input the pass skips it (specs/0052).
            MeetingBriefsRepository::upsert_status(pool, target_meeting_id, "failed", None)
                .await?;
            let count = MeetingBriefsRepository::record_failure(pool, target_meeting_id).await?;
            warn!(
                "prep: brief for {} failed ({}/{} consecutive)",
                target_meeting_id, count, MAX_BRIEF_FAILURES
            );
            Err(e)
        }
```

**Important:** `upsert_status` writes `source_fingerprint = NULL` on failure, so the skip check's fingerprint comparison would never match. Change the failure write to preserve the fingerprint so the give-up rule can bind:

```rust
            MeetingBriefsRepository::upsert_status(
                pool,
                target_meeting_id,
                "failed",
                Some(fingerprint.as_str()),
            )
            .await?;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test --features metal give_up_tests 2>&1 | tail -20
cargo test --features metal 2>&1 | tail -20
```

Expected: PASS (5 new tests), full suite green.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/aggregation/prep_jobs.rs
git commit -m "fix(0052): stop retrying a prep brief after 3 failures against unchanged input"
```

---

### Task 6: The LLM activity registry

**Files:**
- Create: `frontend/src-tauri/src/llm_activity/mod.rs`
- Create: `frontend/src-tauri/src/llm_activity/registry.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (module decl + `.manage`)
- Test: inline `#[cfg(test)]` in `registry.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum TaskKind { PrepBrief, MeetingSummary, ActionItems, NoteEnhancement, AskAI, Rollup }`
  - `pub enum Origin { Background, Foreground }`
  - `pub struct LlmActivityState(pub Arc<LlmTaskRegistry>)`
  - `LlmTaskRegistry::start(&self, kind: TaskKind, origin: Origin, label: impl Into<String>) -> TaskHandle`
  - `TaskHandle::progress(&self, note: impl Into<String>)`, `TaskHandle::finish(self, result: Result<(), String>)`
  - `LlmTaskRegistry::view(&self) -> LlmActivityView`
  - `LlmTaskRegistry::dismiss(&self)`
  - `pub const HISTORY_CAP: usize = 20;`

- [ ] **Step 1: Write the failing test**

Create `frontend/src-tauri/src/llm_activity/registry.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> LlmTaskRegistry {
        LlmTaskRegistry::new()
    }

    #[test]
    fn a_running_task_appears_in_the_view() {
        let r = registry();
        let _task = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        let view = r.view();
        assert_eq!(view.running.len(), 1);
        assert_eq!(view.running[0].label, "Weekly 1:1");
        assert!(!view.has_failure);
    }

    #[test]
    fn a_successful_task_leaves_running_and_records_success() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1")
            .finish(Ok(()));
        let view = r.view();
        assert!(view.running.is_empty());
        assert_eq!(view.history.len(), 1);
        assert!(view.history[0].error.is_none());
        assert!(!view.has_failure, "success must not raise the sticky badge");
    }

    #[test]
    fn a_failed_task_raises_the_sticky_badge() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1")
            .finish(Err("LLM request timed out after 300 seconds".into()));
        let view = r.view();
        assert!(view.has_failure);
        assert_eq!(
            view.history[0].error.as_deref(),
            Some("LLM request timed out after 300 seconds")
        );
    }

    #[test]
    fn dismiss_clears_the_badge_but_keeps_history() {
        let r = registry();
        r.start(TaskKind::PrepBrief, Origin::Background, "x").finish(Err("boom".into()));
        r.dismiss();
        let view = r.view();
        assert!(!view.has_failure);
        assert_eq!(view.history.len(), 1, "dismiss acknowledges, it does not erase");
    }

    /// A job that panics or returns early must not leave a phantom "running" task.
    #[test]
    fn dropping_a_handle_without_finishing_records_a_failure() {
        let r = registry();
        drop(r.start(TaskKind::PrepBrief, Origin::Background, "abandoned"));
        let view = r.view();
        assert!(view.running.is_empty(), "the task must not still look running");
        assert!(view.has_failure);
        assert_eq!(view.history[0].error.as_deref(), Some("Task ended without completing"));
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let r = registry();
        for i in 0..(HISTORY_CAP + 5) {
            r.start(TaskKind::PrepBrief, Origin::Background, format!("task-{i}"))
                .finish(Ok(()));
        }
        let view = r.view();
        assert_eq!(view.history.len(), HISTORY_CAP);
        assert_eq!(view.history[0].label, format!("task-{}", HISTORY_CAP + 4));
    }

    #[test]
    fn foreground_tasks_are_recorded_but_excluded_from_the_background_view() {
        let r = registry();
        let _fg = r.start(TaskKind::AskAI, Origin::Foreground, "Ask AI");
        let _bg = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        let view = r.view();
        assert_eq!(view.running.len(), 1, "only background tasks surface");
        assert_eq!(view.running[0].label, "Weekly 1:1");
        assert_eq!(r.all_running_count(), 2, "but foreground work is still tracked");
    }

    #[test]
    fn progress_updates_the_running_note() {
        let r = registry();
        let task = r.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        task.progress("reading meeting 1 of 2");
        assert_eq!(r.view().running[0].note.as_deref(), Some("reading meeting 1 of 2"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal llm_activity 2>&1 | tail -20
```

Expected: FAIL — the module is not declared and none of the types exist.

- [ ] **Step 3: Implement the registry**

Prepend to `registry.rs`:

```rust
//! In-memory registry of background LLM work (specs/0052).
//!
//! Tracks TASKS, not individual LLM calls: `generate_summary` is the single choke point for
//! every provider call, but it is too low-level to name a unit of work — one summary is many
//! calls. Jobs declare a task; the calls inside report progress against it.
//!
//! Every operation is infallible by construction. A poisoned lock or a full history must
//! never break generation (the specs/0034 extraction contract).

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Finished outcomes retained for the popover. In-memory only: cleared on restart, and
/// deliberately not persisted so prompt/error text never lands on disk.
pub const HISTORY_CAP: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    PrepBrief,
    MeetingSummary,
    ActionItems,
    NoteEnhancement,
    AskAI,
    Rollup,
}

/// `Background` tasks surface in the sidebar indicator. `Foreground` tasks are recorded for
/// observability but already have their own progress UI (ChunkProgressDisplay, the deferred
/// backlog pill, Ask AI's inline progress), so surfacing them would double up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Background,
    Foreground,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningTask {
    pub id: u64,
    pub kind: TaskKind,
    pub label: String,
    pub note: Option<String>,
    /// The meeting this task acts on, when it has one. Required for `PrepBrief` so the
    /// popover's Retry can address the right `meeting_briefs` row.
    pub meeting_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: u64,
    pub kind: TaskKind,
    pub label: String,
    /// `None` on success; the error message on failure.
    pub error: Option<String>,
    /// Carried over from `RunningTask` so a failed prep brief stays retryable.
    pub meeting_id: Option<String>,
}

/// What the frontend renders. Only background work is included.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmActivityView {
    pub running: Vec<RunningTask>,
    /// Newest first, capped at `HISTORY_CAP`.
    pub history: Vec<TaskRecord>,
    /// Sticky until dismissed — the whole point of the feature.
    pub has_failure: bool,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    running: Vec<(Origin, RunningTask)>,
    history: VecDeque<TaskRecord>,
    has_failure: bool,
}

#[derive(Default)]
pub struct LlmTaskRegistry {
    inner: Mutex<Inner>,
}

/// Tauri-managed wrapper so the registry can be shared with command handlers.
pub struct LlmActivityState(pub Arc<LlmTaskRegistry>);

impl LlmTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A poisoned mutex must not take generation down with it — recover the guard.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn start(
        self: &Arc<Self>,
        kind: TaskKind,
        origin: Origin,
        label: impl Into<String>,
    ) -> TaskHandle {
        self.start_for(kind, origin, label, None)
    }

    /// `start` with the meeting this task acts on — use for `PrepBrief` so Retry works.
    pub fn start_for(
        self: &Arc<Self>,
        kind: TaskKind,
        origin: Origin,
        label: impl Into<String>,
        meeting_id: Option<String>,
    ) -> TaskHandle {
        let mut inner = self.lock();
        inner.next_id += 1;
        let id = inner.next_id;
        inner.running.push((
            origin,
            RunningTask { id, kind, label: label.into(), note: None, meeting_id },
        ));
        drop(inner);
        TaskHandle { registry: Arc::clone(self), id, finished: false }
    }

    pub fn view(&self) -> LlmActivityView {
        let inner = self.lock();
        LlmActivityView {
            running: inner
                .running
                .iter()
                .filter(|(origin, _)| *origin == Origin::Background)
                .map(|(_, t)| t.clone())
                .collect(),
            history: inner.history.iter().cloned().collect(),
            has_failure: inner.has_failure,
        }
    }

    /// Every running task regardless of origin — for tests and diagnostics.
    pub fn all_running_count(&self) -> usize {
        self.lock().running.len()
    }

    /// Acknowledge failures. History is retained; only the badge clears.
    pub fn dismiss(&self) {
        self.lock().has_failure = false;
    }

    fn set_note(&self, id: u64, note: String) {
        let mut inner = self.lock();
        if let Some((_, task)) = inner.running.iter_mut().find(|(_, t)| t.id == id) {
            task.note = Some(note);
        }
    }

    fn complete(&self, id: u64, error: Option<String>) {
        let mut inner = self.lock();
        let Some(pos) = inner.running.iter().position(|(_, t)| t.id == id) else {
            return;
        };
        let (origin, task) = inner.running.remove(pos);
        if error.is_some() && origin == Origin::Background {
            inner.has_failure = true;
        }
        inner.history.push_front(TaskRecord {
            id: task.id,
            kind: task.kind,
            label: task.label,
            error,
            meeting_id: task.meeting_id,
        });
        while inner.history.len() > HISTORY_CAP {
            inner.history.pop_back();
        }
    }
}

/// RAII handle. Dropping without `finish` records the task as failed, so a job that panics
/// or returns early can never leave a phantom "running" entry in the sidebar.
pub struct TaskHandle {
    registry: Arc<LlmTaskRegistry>,
    id: u64,
    finished: bool,
}

impl TaskHandle {
    pub fn progress(&self, note: impl Into<String>) {
        self.registry.set_note(self.id, note.into());
    }

    pub fn finish(mut self, result: Result<(), String>) {
        self.finished = true;
        self.registry.complete(self.id, result.err());
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.registry
                .complete(self.id, Some("Task ended without completing".to_string()));
        }
    }
}
```

**Note:** `start` takes `self: &Arc<Self>` so the handle can hold an owned `Arc`. Tests must therefore build `Arc<LlmTaskRegistry>`; update the test helper to:

```rust
    fn registry() -> Arc<LlmTaskRegistry> {
        Arc::new(LlmTaskRegistry::new())
    }
```

- [ ] **Step 4: Create the module root**

Create `frontend/src-tauri/src/llm_activity/mod.rs`:

```rust
//! Background LLM activity tracking (specs/0052).

pub mod commands;
pub mod registry;

pub use registry::{
    LlmActivityState, LlmActivityView, LlmTaskRegistry, Origin, TaskHandle, TaskKind,
};
```

Create a placeholder `frontend/src-tauri/src/llm_activity/commands.rs` (filled in by Task 8):

```rust
//! IPC for the LLM activity indicator (specs/0052). Populated in Task 8.
```

Declare the module in `src/lib.rs` alongside the other `mod` statements:

```rust
mod llm_activity;
```

- [ ] **Step 5: Manage the state**

In `src/lib.rs`, beside the other `.manage(...)` calls (~line 150):

```rust
        .manage(llm_activity::LlmActivityState(std::sync::Arc::new(
            llm_activity::LlmTaskRegistry::new(),
        )))
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test --features metal llm_activity 2>&1 | tail -20
cargo clippy --features metal 2>&1 | tail -10
```

Expected: PASS (8 tests), no clippy warnings.

- [ ] **Step 7: Verify the file-size ratchet**

```bash
cd ../.. && ./scripts/check-file-size.sh
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/llm_activity/ frontend/src-tauri/src/lib.rs
git commit -m "feat(0052): add in-memory LLM task registry with RAII task handles"
```

---

### Task 7: Instrument the background jobs

**Files:**
- Modify: `frontend/src-tauri/src/aggregation/prep_jobs.rs`
- Modify: `frontend/src-tauri/src/action_items/extractor.rs`
- Test: inline `#[cfg(test)]` in `src/aggregation/prep_jobs.rs`

**Interfaces:**
- Consumes: `LlmActivityState`, `LlmTaskRegistry::start`, `TaskHandle::{progress, finish}`, `TaskKind`, `Origin` (Task 6)
- Produces: nothing consumed by later tasks (Task 8 reads the registry directly)

- [ ] **Step 1: Write the failing test**

Add to `src/aggregation/prep_jobs.rs`:

```rust
#[cfg(test)]
mod instrumentation_tests {
    use crate::llm_activity::{LlmTaskRegistry, Origin, TaskKind};
    use std::sync::Arc;

    /// The prep generator must appear as a BACKGROUND task so the sidebar surfaces it —
    /// the whole reason specs/0052 exists is that this work was invisible.
    #[test]
    fn a_prep_task_surfaces_as_background_work() {
        let registry = Arc::new(LlmTaskRegistry::new());
        let task = registry.start(TaskKind::PrepBrief, Origin::Background, "Weekly 1:1");
        task.progress("reading meeting 1 of 2");

        let view = registry.view();
        assert_eq!(view.running.len(), 1);
        assert_eq!(view.running[0].label, "Weekly 1:1");
        assert_eq!(view.running[0].note.as_deref(), Some("reading meeting 1 of 2"));

        task.finish(Err("No summary model is configured".into()));
        let view = registry.view();
        assert!(view.running.is_empty());
        assert!(view.has_failure);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal instrumentation_tests 2>&1 | tail -20
```

Expected: FAIL — `unresolved import` if Task 6's module is not yet visible; otherwise PASS once wiring exists. (This test pins the contract the wiring must satisfy.)

- [ ] **Step 3: Instrument `generate_brief_for_target`**

In `src/aggregation/prep_jobs.rs`, at the start of `generate_brief_for_target` — after `meta` is loaded so the meeting title is available — open a task:

```rust
    let activity = app
        .try_state::<crate::llm_activity::LlmActivityState>()
        .map(|s| Arc::clone(&s.0));
    let task = activity.as_ref().map(|r| {
        r.start_for(
            crate::llm_activity::TaskKind::PrepBrief,
            crate::llm_activity::Origin::Background,
            format!("Preparing brief — {}", meta.title),
            Some(target_meeting_id.to_string()),
        )
    });
```

Add `use std::sync::Arc;` to the imports.

Forward engine stages into the handle by wrapping the existing `on_progress` closure passed to `execute_pre_call_prep`:

```rust
    let stage_task = task.as_ref();
    let on_progress = |stage: Stage| {
        if let Some(t) = stage_task {
            t.progress(format!("{stage:?}"));
        }
        on_progress(stage);
    };
```

At each of the function's return points, finish the task:

- the `prior.is_empty()` early return → `if let Some(t) = task { t.finish(Ok(())); }`
- the provider-resolution `Err` arms → `t.finish(Err(e.to_string()))`
- the `Ok(answer)` arm → `t.finish(Ok(()))`
- the `Err(e)` arm → `t.finish(Err(format!("{e:#}")))`

Any path that returns without finishing is still safe — `Drop` records it as failed.

- [ ] **Step 4: Instrument action-item extraction**

Read the extraction entry point first (`grep -n "pub async fn" src/action_items/extractor.rs`) to find the outermost function and whether an `AppHandle` is in scope. Then open a task at its top:

```rust
    let task = activity.as_ref().map(|r| {
        r.start_for(
            crate::llm_activity::TaskKind::ActionItems,
            crate::llm_activity::Origin::Background,
            format!("Extracting action items — {meeting_title}"),
            Some(meeting_id.to_string()),
        )
    });
```

and finish it on every return path exactly as in Step 3 (`t.finish(Ok(()))` / `t.finish(Err(format!("{e:#}")))`).

If no `AppHandle` is in scope, thread `activity: Option<Arc<LlmTaskRegistry>>` in as a parameter rather than reaching for global state — the extractor is called from several places and a global would couple them.

**Scope note:** this task instruments the two `Origin::Background` producers. `MeetingSummary`, `NoteEnhancement`, `AskAI`, and `Rollup` are `Origin::Foreground` (they already have their own progress UI) and are **not** instrumented here — the `TaskKind` variants exist so they can be added without a type change. This is a deliberate narrowing of the spec's task-list item 7; the sidebar surfaces only background work either way.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test --features metal 2>&1 | tail -20
cargo clippy --features metal 2>&1 | tail -10
```

Expected: full suite green, no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/aggregation/prep_jobs.rs \
        frontend/src-tauri/src/action_items/extractor.rs
git commit -m "feat(0052): report prep-brief and action-item work to the activity registry"
```

---

### Task 8: IPC commands and the change event

**Files:**
- Modify: `frontend/src-tauri/src/llm_activity/commands.rs`
- Modify: `frontend/src-tauri/src/llm_activity/registry.rs` (emit on transition)
- Modify: `frontend/src-tauri/src/registry.rs` (register commands)
- Test: inline `#[cfg(test)]` in `commands.rs`

**Interfaces:**
- Consumes: everything from Task 6, plus `MeetingBriefsRepository::reset_failures` (Task 1)
- Produces:
  - `api_llm_activity_snapshot() -> LlmActivityView`
  - `api_llm_activity_dismiss()`
  - `api_llm_activity_retry(meeting_id: String)`
  - Event name constant `pub const LLM_ACTIVITY_EVENT: &str = "llm-activity-changed";`

- [ ] **Step 1: Write the failing test**

Replace the placeholder `commands.rs` with a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend listens on this exact string; changing it silently breaks the indicator.
    #[test]
    fn the_event_name_is_stable() {
        assert_eq!(LLM_ACTIVITY_EVENT, "llm-activity-changed");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --features metal llm_activity 2>&1 | tail -20
```

Expected: FAIL — `cannot find value 'LLM_ACTIVITY_EVENT'`.

- [ ] **Step 3: Implement the commands**

Prepend to `commands.rs`:

```rust
//! IPC for the LLM activity indicator (specs/0052).

use crate::llm_activity::{LlmActivityState, LlmActivityView};
use crate::state::AppState;
use tauri::{AppHandle, Emitter, Manager, State};

/// Emitted on every task transition with the full view. The frontend also pulls a snapshot
/// on mount, so a missed event self-heals on the next transition.
pub const LLM_ACTIVITY_EVENT: &str = "llm-activity-changed";

/// Emit the current view. Best-effort: a failed emit must never break generation.
pub fn emit_activity(app: &AppHandle, view: &LlmActivityView) {
    let _ = app.emit(LLM_ACTIVITY_EVENT, view);
}

#[tauri::command]
pub async fn api_llm_activity_snapshot(
    state: State<'_, LlmActivityState>,
) -> Result<LlmActivityView, String> {
    Ok(state.0.view())
}

#[tauri::command]
pub async fn api_llm_activity_dismiss(
    app: AppHandle,
    state: State<'_, LlmActivityState>,
) -> Result<(), String> {
    state.0.dismiss();
    emit_activity(&app, &state.0.view());
    Ok(())
}

/// Clear a prep brief's failure counter and force one regeneration pass.
///
/// Retry is prep-brief-only: prep is the one background task with a durable, addressable
/// artifact (a `meeting_briefs` row keyed by meeting) and a give-up rule to clear.
#[tauri::command]
pub async fn api_llm_activity_retry(app: AppHandle, meeting_id: String) -> Result<(), String> {
    let pool = app
        .try_state::<AppState>()
        .ok_or_else(|| "App state unavailable".to_string())?
        .db_manager
        .pool()
        .clone();

    crate::database::repositories::meeting_brief::MeetingBriefsRepository::reset_failures(
        &pool,
        &meeting_id,
    )
    .await
    .map_err(|e| format!("Could not clear the failure count: {e}"))?;

    crate::aggregation::prep_jobs::run_prep_pass(&app).await;
    Ok(())
}
```

- [ ] **Step 4: Emit on every transition**

The registry has no `AppHandle`, so give it an optional emitter set at startup. In `registry.rs`, add to `LlmTaskRegistry`:

```rust
    /// Set once at startup so transitions can notify the frontend. Optional because the
    /// registry is constructed before the app handle exists (and in tests there is none).
    app: Mutex<Option<tauri::AppHandle>>,
```

Add:

```rust
    pub fn attach(&self, app: tauri::AppHandle) {
        *self.app.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
    }

    fn notify(&self) {
        let handle = self
            .app
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(app) = handle {
            crate::llm_activity::commands::emit_activity(&app, &self.view());
        }
    }
```

Call `self.notify()` at the end of `start`, `set_note`, `complete`, and `dismiss` — **after** releasing the inner lock in each (`notify` calls `view()`, which re-locks; holding the guard would deadlock).

In `src/lib.rs` `setup`, after the state is managed:

```rust
            if let Some(activity) = _app.handle().try_state::<llm_activity::LlmActivityState>() {
                activity.0.attach(_app.handle().clone());
            }
```

- [ ] **Step 5: Register the commands**

In `src/registry.rs`, add `llm_activity` to the `use crate::{...}` list and append a section to `tauri::generate_handler![...]`:

```rust
        // Background LLM activity indicator (specs/0052)
        llm_activity::commands::api_llm_activity_snapshot,
        llm_activity::commands::api_llm_activity_dismiss,
        llm_activity::commands::api_llm_activity_retry,
```

- [ ] **Step 6: Run the tests and build**

```bash
cargo test --features metal 2>&1 | tail -20
cargo clippy --features metal 2>&1 | tail -10
```

Expected: full suite green, no clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add frontend/src-tauri/src/llm_activity/ frontend/src-tauri/src/registry.rs \
        frontend/src-tauri/src/lib.rs
git commit -m "feat(0052): expose LLM activity over IPC with a change event"
```

---

### Task 9: Sidebar activity row and history popover

**Files:**
- Create: `frontend/src/contexts/LlmActivityProvider.tsx`
- Create: `frontend/src/components/LlmActivity/LlmActivityRow.tsx`
- Create: `frontend/src/components/LlmActivity/LlmActivityPopover.tsx`
- Create: `frontend/src/components/LlmActivity/__tests__/LlmActivityRow.test.tsx`
- Modify: `frontend/src/app/layout.tsx`, `frontend/src/components/Sidebar/index.tsx`

**Interfaces:**
- Consumes: `api_llm_activity_snapshot`, `api_llm_activity_dismiss`, `api_llm_activity_retry`, the `llm-activity-changed` event (Task 8)
- Produces: `useLlmActivity(): LlmActivityView & { dismiss, retry }`

- [ ] **Step 1: Write the failing test**

Create `frontend/src/components/LlmActivity/__tests__/LlmActivityRow.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { LlmActivityRow } from '../LlmActivityRow';

vi.mock('@/contexts/LlmActivityProvider', () => ({
  useLlmActivity: () => mockValue,
}));

let mockValue: any;

describe('LlmActivityRow', () => {
  it('shows the running task label', () => {
    mockValue = {
      running: [{ id: 1, kind: 'prepBrief', label: 'Preparing brief — Weekly 1:1', note: null }],
      history: [],
      hasFailure: false,
      dismiss: vi.fn(),
      retry: vi.fn(),
    };
    render(<LlmActivityRow />);
    expect(screen.getByText('Preparing brief — Weekly 1:1')).toBeInTheDocument();
  });

  it('shows an idle label when nothing is running', () => {
    mockValue = { running: [], history: [], hasFailure: false, dismiss: vi.fn(), retry: vi.fn() };
    render(<LlmActivityRow />);
    expect(screen.getByText('Background AI idle')).toBeInTheDocument();
  });

  it('shows a sticky failure count when a task has failed', () => {
    mockValue = {
      running: [],
      history: [{ id: 1, kind: 'prepBrief', label: 'Weekly 1:1', error: 'timed out' }],
      hasFailure: true,
      dismiss: vi.fn(),
      retry: vi.fn(),
    };
    render(<LlmActivityRow />);
    expect(screen.getByText('1 task failed')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd frontend && pnpm test LlmActivityRow 2>&1 | tail -20
```

Expected: FAIL — cannot resolve `../LlmActivityRow`.

- [ ] **Step 3: Implement the provider**

Create `frontend/src/contexts/LlmActivityProvider.tsx`:

```tsx
'use client';

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

export type LlmTaskKind =
  | 'prepBrief' | 'meetingSummary' | 'actionItems'
  | 'noteEnhancement' | 'askAI' | 'rollup';

export interface RunningTask {
  id: number;
  kind: LlmTaskKind;
  label: string;
  note: string | null;
}

export interface TaskRecord {
  id: number;
  kind: LlmTaskKind;
  label: string;
  /** null on success; the error message on failure. */
  error: string | null;
}

export interface LlmActivityView {
  running: RunningTask[];
  history: TaskRecord[];
  hasFailure: boolean;
}

const EMPTY: LlmActivityView = { running: [], history: [], hasFailure: false };

interface LlmActivityValue extends LlmActivityView {
  dismiss: () => Promise<void>;
  retry: (meetingId: string) => Promise<void>;
}

const Ctx = createContext<LlmActivityValue | null>(null);

/**
 * Background LLM activity (spec 0052). Mounted once at app level. Pulls a snapshot on mount
 * and then follows `llm-activity-changed`, so a missed event self-heals on the next
 * transition.
 */
export function LlmActivityProvider({ children }: { children: ReactNode }) {
  const [view, setView] = useState<LlmActivityView>(EMPTY);

  useEffect(() => {
    let cancelled = false;
    invoke<LlmActivityView>('api_llm_activity_snapshot')
      .then((v) => { if (!cancelled) setView(v); })
      .catch(() => { /* indicator is best-effort; never surface a toast for this */ });

    const unlisten = safeListen<LlmActivityView>('llm-activity-changed', (e) => {
      setView(e.payload);
    });
    return () => { cancelled = true; void unlisten.then((f) => f()); };
  }, []);

  const dismiss = useCallback(async () => {
    try { await invoke('api_llm_activity_dismiss'); } catch { /* best-effort */ }
  }, []);

  const retry = useCallback(async (meetingId: string) => {
    try { await invoke('api_llm_activity_retry', { meetingId }); } catch { /* best-effort */ }
  }, []);

  return <Ctx.Provider value={{ ...view, dismiss, retry }}>{children}</Ctx.Provider>;
}

export function useLlmActivity(): LlmActivityValue {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useLlmActivity must be used within an LlmActivityProvider');
  return ctx;
}
```

Confirm `safeListen`'s exact signature in `frontend/src/lib/safe-listen.ts` first and match it — the shape above assumes it returns a promise of an unlisten function, mirroring `useDeferredBacklog`.

- [ ] **Step 4: Implement the row**

Create `frontend/src/components/LlmActivity/LlmActivityRow.tsx`:

```tsx
'use client';

import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { useLlmActivity } from '@/contexts/LlmActivityProvider';
import { LlmActivityPopover } from './LlmActivityPopover';

/**
 * Sidebar footer row for background LLM work (spec 0052). Ambient, not an interruption:
 * this runs every 30 minutes, including during meetings. Failures stay visible until
 * acknowledged — a background failure that clears itself teaches the user nothing.
 *
 * Deliberately NOT bottom-center: DeferredBacklogIndicator owns that region.
 */
export function LlmActivityRow() {
  const { running, history, hasFailure } = useLlmActivity();
  const [open, setOpen] = useState(false);

  const failures = history.filter((h) => h.error !== null).length;
  const active = running[0];

  return (
    <div className="relative px-3 py-2 border-t border-border">
      {open && <LlmActivityPopover onClose={() => setOpen(false)} />}
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label="Background AI activity"
        className="flex w-full items-center gap-2 text-left text-xs text-muted-foreground hover:text-foreground"
      >
        {active ? (
          <Loader2 size={12} className="animate-spin shrink-0" aria-hidden />
        ) : (
          <span
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${hasFailure ? 'bg-red-500' : 'bg-muted-foreground/40'}`}
            aria-hidden
          />
        )}
        <span className="truncate">
          {active
            ? active.label
            : hasFailure
              ? `${failures} task${failures === 1 ? '' : 's'} failed`
              : 'Background AI idle'}
        </span>
      </button>
    </div>
  );
}
```

- [ ] **Step 5: Implement the popover**

Create `frontend/src/components/LlmActivity/LlmActivityPopover.tsx`:

```tsx
'use client';

import { useLlmActivity } from '@/contexts/LlmActivityProvider';

/**
 * Recent background LLM outcomes (spec 0052). Retry is prep-brief-only — prep is the one
 * background task with a durable, addressable artifact and a give-up counter to clear.
 */
export function LlmActivityPopover({ onClose }: { onClose: () => void }) {
  const { history, hasFailure, dismiss, retry } = useLlmActivity();

  return (
    <div className="absolute bottom-full left-2 right-2 mb-2 z-50 rounded-lg border border-border bg-popover p-3 shadow-xl">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-medium">Background AI</span>
        <div className="flex gap-2">
          {hasFailure && (
            <button type="button" onClick={() => void dismiss()} className="text-xs text-muted-foreground hover:text-foreground">
              Dismiss
            </button>
          )}
          <button type="button" onClick={onClose} className="text-xs text-muted-foreground hover:text-foreground">
            Close
          </button>
        </div>
      </div>

      {history.length === 0 ? (
        <p className="text-xs text-muted-foreground">No recent activity.</p>
      ) : (
        <ul className="max-h-64 space-y-1.5 overflow-y-auto">
          {history.map((item) => (
            <li key={item.id} className="text-xs">
              <div className="flex items-start gap-2">
                <span className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-full ${item.error ? 'bg-red-500' : 'bg-green-500'}`} aria-hidden />
                <div className="min-w-0 flex-1">
                  <div className="truncate">{item.label}</div>
                  {item.error && <div className="text-red-500 break-words">{item.error}</div>}
                </div>
                {item.error && item.kind === 'prepBrief' && item.meetingId && (
                  <button type="button" onClick={() => void retry(item.meetingId!)} className="shrink-0 text-muted-foreground hover:text-foreground">
                    Retry
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
```

`meetingId` comes from `TaskRecord.meeting_id`, populated by `start_for` in Task 7's prep instrumentation. Add it to the `TaskRecord` interface in the provider:

```ts
export interface TaskRecord {
  id: number;
  kind: LlmTaskKind;
  label: string;
  error: string | null;
  meetingId: string | null;
}
```

and to `RunningTask` as `meetingId: string | null`.

- [ ] **Step 6: Mount the provider and the row**

In `frontend/src/app/layout.tsx`, import and wrap alongside the existing providers (inside `DeferredBacklogProvider`, ~line 272):

```tsx
import { LlmActivityProvider } from '@/contexts/LlmActivityProvider'
```

In `frontend/src/components/Sidebar/index.tsx`, render `<LlmActivityRow />` at the bottom of the sidebar's nav container.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cd frontend && pnpm test LlmActivityRow 2>&1 | tail -20
pnpm lint 2>&1 | tail -10
pnpm test 2>&1 | tail -20
```

Expected: 3 new tests pass, lint clean, full suite green.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/contexts/LlmActivityProvider.tsx \
        frontend/src/components/LlmActivity/ \
        frontend/src/app/layout.tsx frontend/src/components/Sidebar/index.tsx
git commit -m "feat(0052): surface background LLM activity in the sidebar"
```

---

## Final verification

- [ ] **Full gate** (the Definition of Done in `CLAUDE.md`)

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo check --features metal && cargo clippy --features metal && cargo test --features metal
cd .. && pnpm lint && pnpm test
cd .. && ./scripts/check-file-size.sh
```

- [ ] **Manual smoke** (`./dev-vinyl.sh`, Ollama as the summary provider)

1. Sidebar shows `· Background AI idle` at rest.
2. With an upcoming recurring meeting that has ≥1 prior occurrence with a summary, trigger a prep pass; the row shows a spinner and the meeting title, then clears on success.
3. Point Vinyl at a nonexistent model name to force failure; confirm the red dot persists across navigation, the popover shows the error text, **Dismiss** clears the badge while keeping history, and **Retry** re-runs.
4. Let the forced failure repeat: the 4th pass logs `prep: skipping … 3 consecutive failures` and issues no LLM request.
5. In the Ollama server log, confirm requests now carry `max_tokens` and none run to the 300s deadline.
6. With `OLLAMA_CONTEXT_LENGTH=65536`, confirm the logged budget line reads ~65,236 rather than ~261,844.

- [ ] **Open the PR**

```bash
git push -u origin feat/0052-llm-activity-and-context-detection
gh pr create --title "0052: background LLM activity indicator + served-context detection" --body "$(cat <<'BODY'
Implements `specs/0052-llm-activity-and-context-detection.md`.

Background pre-call-prep generation was burning GPU during meetings with no UI signal.
Three defects compounded:

- Ollama requests shipped with no `max_tokens` (`provider_config.rs:75` hardcodes `None`;
  `ChatRequest` skips the field), so generation was bounded only by the 300s client timeout.
- `prep_jobs.rs` recorded `'failed'` with no counter, so a broken brief retried every 30
  minutes forever.
- The context budget came from `/api/show`'s architectural max (262k for gemma4) rather
  than the served size, so summarization never chunked and long meetings were silently
  context-shifted.

Adds per-call-site `max_tokens` caps, a give-up-at-3 rule for prep briefs, a
`min(architectural, served)` context clamp fed by `/api/ps`, and an in-memory LLM task
registry surfaced as a sidebar activity row with sticky failures.

Prep still runs during recording — that is an explicit owner decision; the indicator is the
mitigation.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
```
