//! Action-items lifecycle test (specs/0034 task 8): mocked-LLM extract → user
//! mutations → re-extract diff assertions, over a REAL migrated database.
//!
//! Follows the `db_lifecycle.rs` pattern: a temp-file SQLite brought up through the
//! app's real migration path (`DatabaseManager::new`), driving the same repository +
//! diff calls the background trigger (`action_items::run_extraction`) and the manual
//! `api_extract_action_items` command wrap. `run_extraction` itself needs an
//! `AppHandle` (for the `action-items-updated` event), so [`extract_pass`] mirrors its
//! orchestration — ledger skip-check → `compute_diff` → `replace_extracted` — with
//! candidate fixtures standing in for LLM output. The LLM boundary itself
//! (prompt/parse/retry) is unit-tested in `src/action_items/extractor.rs`.
//!
//! Verification-matrix coverage (specs/0034):
//! - rows 1–8: this file (integration level, through the repo transaction);
//! - row 9 (owner resolution): unit level in `extractor.rs`
//!   (`resolves_roster_name_unknown_and_me`) + persisted end-to-end in row 1 here;
//! - row 10 (fenced/prose/invalid JSON): unit level in `extractor.rs` (parse + retry
//!   seam tests — a double parse failure is `Err`, so nothing here ever writes);
//! - row 11 (cascades): repository level in `database/repositories/action_item.rs`
//!   (`meeting_delete_cascades_items_and_ledger`,
//!   `person_delete_nulls_assignee_but_keeps_item`).
//!
//! Run with:
//!   cargo test --features metal --test action_items_lifecycle -- --nocapture

mod common;

use app_lib::action_items::diff::{self, compute_diff, ResolvedCandidate};
use app_lib::action_items::extractor::{resolve_candidates, ActionItemCandidate, OwnerIdentity};
use app_lib::action_items::skip_zero_candidate_deletes;
use app_lib::database::repositories::action_item::{ActionItem, ActionItemsRepository};
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::meeting_participant::MeetingParticipantsRepository;
use app_lib::database::repositories::people::PeopleRepository;
use common::fresh_db;
use sqlx::SqlitePool;

/// One extraction pass exactly as `run_extraction` orchestrates it (minus the LLM call
/// and the Tauri event): ledger fingerprint skip-check → load existing rows →
/// `compute_diff` → zero-candidate safety valve → atomic `replace_extracted`. Returns
/// `false` when nothing was applied: a ledger fingerprint no-op (verification row 2) or
/// a suspect zero-candidate run (2026-07 incident).
async fn extract_pass(
    pool: &SqlitePool,
    meeting_id: &str,
    summary_markdown: &str,
    user_notes: Option<&str>,
    candidates: &[ResolvedCandidate],
) -> bool {
    let fingerprint = diff::extraction_fingerprint(summary_markdown, user_notes);
    if let Some(ledger) = ActionItemsRepository::get_extraction(pool, meeting_id)
        .await
        .expect("read extraction ledger")
    {
        if ledger.summary_fingerprint == fingerprint {
            return false;
        }
    }

    let existing = ActionItemsRepository::get_for_meeting(pool, meeting_id)
        .await
        .expect("load existing items");
    let plan = compute_diff(&existing, candidates);
    // The zero-candidate safety valve, exactly where `run_extraction` applies it — the
    // REAL guard, not a mirror: a suspect empty extraction applies nothing and leaves
    // the ledger untouched so a later run retries.
    if skip_zero_candidate_deletes(meeting_id, candidates.len() as u32, &plan) {
        return false;
    }
    let written = ActionItemsRepository::replace_extracted(
        pool,
        meeting_id,
        &plan,
        &fingerprint,
        "ollama",
        "llama3.2:3b",
        candidates.len() as u32,
    )
    .await
    .expect("apply extraction diff");
    assert!(
        written.is_some(),
        "meeting exists throughout these scenarios — apply must not abort"
    );
    true
}

fn cand(description: &str) -> ResolvedCandidate {
    ResolvedCandidate {
        description: description.to_string(),
        assignee_person_id: None,
        assignee_is_self: false,
        assignee_raw: None,
        due_hint: None,
        due_date: None,
    }
}

async fn create_meeting(pool: &SqlitePool) -> String {
    MeetingsRepository::create_meeting(pool, Some("Weekly Sync".into()), None, None, None, None)
        .await
        .expect("create_meeting")
}

async fn items(pool: &SqlitePool, meeting_id: &str) -> Vec<ActionItem> {
    ActionItemsRepository::get_for_meeting(pool, meeting_id)
        .await
        .expect("get_for_meeting")
}

/// Verification row 1 — fresh meeting, 3 candidates → 3 rows inserted
/// (`source='extracted'`, `status='open'`), ledger written. Candidates go through the
/// REAL owner resolution against a REAL roster, so assignee persistence (person FK /
/// raw fallback / self flag — row 9's integration face) is asserted end-to-end.
#[tokio::test]
async fn fresh_extraction_inserts_rows_and_ledger() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    // Roster: Alice only. "Bob" is off-roster; "me" is the app owner.
    let alice = PeopleRepository::create(pool, "Alice", Some("alice@example.com"), None, None)
        .await
        .expect("create person");
    MeetingParticipantsRepository::add(pool, &meeting_id, &alice.id, "manual")
        .await
        .expect("add participant");
    let roster = MeetingParticipantsRepository::list(pool, &meeting_id)
        .await
        .expect("list roster");

    // Raw LLM-shaped candidates → real resolution → the diff/persistence shape.
    let raw = vec![
        ActionItemCandidate {
            description: "Send the revised deck to the client".into(),
            assignee: Some("Alice".into()),
            due: Some("Friday".into()),
        },
        ActionItemCandidate {
            description: "Draft the beta announcement".into(),
            assignee: Some("Bob".into()),
            due: None,
        },
        ActionItemCandidate {
            description: "Book the retro room".into(),
            assignee: Some("me".into()),
            due: None,
        },
    ];
    let candidates = resolve_candidates(raw, &roster, &OwnerIdentity::default());

    let summary = "## Action Items\n- deck, announcement, room";
    assert!(extract_pass(pool, &meeting_id, summary, None, &candidates).await);

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 3, "three candidates → three rows");
    for row in &rows {
        assert_eq!(row.source, "extracted");
        assert_eq!(row.status, "open");
        assert!(!row.user_edited);
        assert!(row.id.starts_with("ai-"));
    }
    let deck = rows
        .iter()
        .find(|r| r.description.contains("deck"))
        .unwrap();
    assert_eq!(deck.assignee_person_id.as_deref(), Some(alice.id.as_str()));
    assert_eq!(deck.due_hint.as_deref(), Some("Friday"));
    let announce = rows
        .iter()
        .find(|r| r.description.contains("announcement"))
        .unwrap();
    assert_eq!(
        announce.assignee_person_id, None,
        "off-roster name is not a FK"
    );
    assert_eq!(announce.assignee_raw.as_deref(), Some("Bob"));
    let room = rows
        .iter()
        .find(|r| r.description.contains("room"))
        .unwrap();
    assert!(room.assignee_is_self, "\"me\" → assignee_is_self");
    assert_eq!(room.assignee_person_id, None);

    let ledger = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .expect("read ledger")
        .expect("ledger row written");
    assert_eq!(
        ledger.summary_fingerprint,
        diff::extraction_fingerprint(summary, None)
    );
    assert_eq!(ledger.item_count, 3);
    assert_eq!(ledger.model_provider, "ollama");
    assert_eq!(ledger.model_name, "llama3.2:3b");
}

/// Verification row 2 — re-running against an identical summary fingerprint is a
/// no-op: zero row churn, `updated_at` untouched, ledger untouched.
#[tokio::test]
async fn identical_summary_rerun_is_a_noop() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    let summary = "## Action Items\n- send the deck\n- book the room";
    let notes = Some("remember: deck first");
    let candidates = vec![cand("Send the deck"), cand("Book the room")];
    assert!(extract_pass(pool, &meeting_id, summary, notes, &candidates).await);

    let rows_before = items(pool, &meeting_id).await;
    let ledger_before = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();

    // Same summary + notes → fingerprint match → the pass never runs.
    assert!(
        !extract_pass(pool, &meeting_id, summary, notes, &candidates).await,
        "identical fingerprint must short-circuit"
    );

    let rows_after = items(pool, &meeting_id).await;
    assert_eq!(
        format!("{rows_before:?}"),
        format!("{rows_after:?}"),
        "zero churn: ids, content, and updated_at all byte-identical"
    );
    let ledger_after = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        format!("{ledger_before:?}"),
        format!("{ledger_after:?}"),
        "ledger (incl. extracted_at) untouched on a no-op"
    );

    // Changing the NOTES alone invalidates the fingerprint (notes are extraction input).
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            summary,
            Some("edited notes"),
            &candidates
        )
        .await
    );
}

/// Regression (cross-path fingerprint, acceptance #4): the auto post-summary pass and a
/// manual "Scan again" both extract from the STORED title-stripped `$.markdown` — the
/// byte equality of those two production inputs is pinned in `summary/service.rs`
/// (`auto_then_manual_extraction_fingerprints_match_on_unchanged_summary`). HERE we pin
/// the consequence end to end: with equal inputs, auto-then-manual on an unchanged
/// summary is a ledger no-op, not a second full extraction.
#[tokio::test]
async fn auto_then_manual_pass_on_unchanged_summary_is_ledger_noop() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    // The stored representation service.rs writes at $.markdown (title already stripped).
    let stored_markdown = "## Action Items\n- send the deck";
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            stored_markdown,
            None,
            &[cand("send the deck")]
        )
        .await,
        "auto pass extracts"
    );
    assert!(
        !extract_pass(
            pool,
            &meeting_id,
            stored_markdown,
            None,
            &[cand("send the deck")]
        )
        .await,
        "manual pass over the identical stored summary is a ledger no-op"
    );
}

/// Verification row 3 — a COMPLETED item absorbs its reworded (≥ 0.7 similarity)
/// candidate: untouched (same id, still completed, original text), zero duplicates.
#[tokio::test]
async fn completed_item_survives_reworded_rerun() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    let original = "send the quarterly deck to alice by friday";
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand(original), cand("book the room")],
        )
        .await
    );
    let item_a = items(pool, &meeting_id)
        .await
        .into_iter()
        .find(|r| r.description == original)
        .unwrap();

    // User completes A.
    let completed = ActionItemsRepository::set_status(pool, &item_a.id, "completed")
        .await
        .unwrap()
        .unwrap();
    assert!(completed.completed_at.is_some());

    // Regenerated summary rewords A (6 shared tokens, union 8 → 0.75 ≥ 0.7).
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2",
            None,
            &[
                cand("send the quarterly deck to alice"),
                cand("book the room")
            ],
        )
        .await
    );

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 2, "no duplicate of the completed item");
    let a_after = rows
        .iter()
        .find(|r| r.id == item_a.id)
        .expect("A kept, same id");
    assert_eq!(a_after.status, "completed");
    assert_eq!(
        a_after.description, original,
        "text exactly as the user left it"
    );
    assert_eq!(a_after.completed_at, completed.completed_at);
    assert_eq!(
        a_after.updated_at, completed.updated_at,
        "row not rewritten"
    );
}

/// Verification row 4 — a USER-EDITED item absorbs a candidate carrying its original
/// phrasing: untouched (`user_edited = 1`, edited text kept), no duplicate.
#[tokio::test]
async fn edited_item_survives_rerun_with_original_phrasing() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    let original = "send the deck to alice";
    assert!(extract_pass(pool, &meeting_id, "summary v1", None, &[cand(original)]).await);
    let item_b = items(pool, &meeting_id).await.remove(0);

    // User edits B's text (the real command path: update_content sets user_edited=1).
    let edited_text = "send the deck to alice please";
    let edited = ActionItemsRepository::update_content(
        pool,
        &item_b.id,
        edited_text,
        None,
        false,
        None,
        None,
        None,
        &diff::content_key(edited_text),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(edited.user_edited);

    // The regenerated summary still contains the ORIGINAL phrasing
    // (5 shared tokens, union 6 → 0.833 ≥ 0.7 → absorbed by the protected row).
    assert!(extract_pass(pool, &meeting_id, "summary v2", None, &[cand(original)]).await);

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 1, "no duplicate with the original phrasing");
    assert_eq!(rows[0].id, item_b.id);
    assert_eq!(rows[0].description, edited_text, "the user's edit wins");
    assert!(rows[0].user_edited);
    assert_eq!(rows[0].updated_at, edited.updated_at, "row not rewritten");
}

/// Verification row 5 — a DISMISSED item stays dismissed; the candidate reappearing in
/// the new summary is dropped, not resurrected.
#[tokio::test]
async fn dismissed_item_is_not_resurrected() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    let text = "follow up with legal";
    assert!(extract_pass(pool, &meeting_id, "summary v1", None, &[cand(text)]).await);
    let item_c = items(pool, &meeting_id).await.remove(0);
    ActionItemsRepository::set_status(pool, &item_c.id, "dismissed")
        .await
        .unwrap()
        .unwrap();

    // New summary still contains C verbatim.
    assert!(extract_pass(pool, &meeting_id, "summary v2", None, &[cand(text)]).await);

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 1, "dismissal is persistent — no re-insert");
    assert_eq!(rows[0].id, item_c.id);
    assert_eq!(rows[0].status, "dismissed");
}

/// Verification row 6 — a PRISTINE item the new summary omits is deleted (machine-owned
/// content superseded by the new summary).
#[tokio::test]
async fn pristine_item_omitted_is_deleted() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand("book the conference room"), cand("send the deck")],
        )
        .await
    );

    // The regenerated summary no longer supports the room booking.
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2",
            None,
            &[cand("send the deck")]
        )
        .await
    );

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].description, "send the deck");
}

/// Verification row 7 — a PRISTINE item rephrased (Jaccard ≥ 0.7) in the new summary is
/// updated in place: same id, `created_at` and `status` preserved, content rewritten.
#[tokio::test]
async fn pristine_item_rephrased_updates_in_place() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand("send the deck to alice by friday")],
        )
        .await
    );
    let item_e = items(pool, &meeting_id).await.remove(0);

    // Rephrased: drop "the" (∩6/∪7 ≈ 0.857 ≥ 0.7) and now with a due hint.
    let mut rephrased = cand("send deck to alice by friday");
    rephrased.due_hint = Some("friday".into());
    assert!(extract_pass(pool, &meeting_id, "summary v2", None, &[rephrased]).await);

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 1, "update in place, not delete+insert");
    assert_eq!(rows[0].id, item_e.id, "same id");
    assert_eq!(
        rows[0].created_at, item_e.created_at,
        "created_at preserved"
    );
    assert_eq!(rows[0].status, "open");
    assert_eq!(rows[0].description, "send deck to alice by friday");
    assert_eq!(rows[0].due_hint.as_deref(), Some("friday"));
    assert_eq!(
        rows[0].content_key,
        diff::content_key("send deck to alice by friday"),
        "content_key recomputed for the new text"
    );
}

/// Verification row 8 — a MANUAL item on the meeting survives any re-run: never
/// updated, deleted, or duplicated, whether or not a candidate matches it.
#[tokio::test]
async fn manual_item_survives_any_rerun() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand("send the deck")]
        )
        .await
    );
    let manual = ActionItemsRepository::create(
        pool,
        Some(&meeting_id),
        &cand("water the office plants"),
        "manual",
        &diff::content_key("water the office plants"),
    )
    .await
    .unwrap();

    // Re-run whose candidates MATCH the manual item → dropped, no duplicate.
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2",
            None,
            &[cand("send the deck"), cand("water the office plants")],
        )
        .await
    );
    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 2, "no duplicate of the manual item");
    let m = rows
        .iter()
        .find(|r| r.id == manual.id)
        .expect("manual kept");
    assert_eq!(m.source, "manual");
    assert_eq!(m.updated_at, manual.updated_at, "manual row not rewritten");

    // Re-run that OMITS it entirely → still kept (protected, never deleted).
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v3",
            None,
            &[cand("send the deck")]
        )
        .await
    );
    let rows = items(pool, &meeting_id).await;
    assert!(
        rows.iter().any(|r| r.id == manual.id),
        "manual row survives omission"
    );
}

/// Acceptance criterion 3, whole-scenario: complete A + edit B + dismiss C, then one
/// regeneration whose candidates reword A, carry B's original phrasing, repeat C, omit
/// pristine D, rephrase pristine E, and add new F — all in ONE diff transaction. The
/// three user-touched items come out exactly as the user left them (same ids, status,
/// text), zero duplicates; D is deleted, E updated in place, F inserted.
#[tokio::test]
async fn regeneration_leaves_user_touched_items_exactly_as_left() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    let a = "send the quarterly deck to alice by friday";
    let b = "book the offsite venue for the retreat";
    let c = "follow up with legal";
    let d = "update the wiki page";
    let e = "schedule the beta review with the pilot group";
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand(a), cand(b), cand(c), cand(d), cand(e)],
        )
        .await
    );
    let by_desc = |rows: &[ActionItem], text: &str| -> ActionItem {
        rows.iter().find(|r| r.description == text).unwrap().clone()
    };
    let rows = items(pool, &meeting_id).await;
    let (item_a, item_b, item_c, item_d, item_e) = (
        by_desc(&rows, a),
        by_desc(&rows, b),
        by_desc(&rows, c),
        by_desc(&rows, d),
        by_desc(&rows, e),
    );

    // (a) complete A, (b) edit B, (c) dismiss C.
    ActionItemsRepository::set_status(pool, &item_a.id, "completed")
        .await
        .unwrap()
        .unwrap();
    // 6 original tokens + {in, tahoe} → the original phrasing scores 6/8 = 0.75 vs the
    // edit, so the protected row absorbs it (≥ 0.7).
    let b_edited = "book the offsite venue for the retreat in tahoe";
    ActionItemsRepository::update_content(
        pool,
        &item_b.id,
        b_edited,
        None,
        true,
        None,
        Some("next month"),
        None,
        &diff::content_key(b_edited),
    )
    .await
    .unwrap()
    .unwrap();
    ActionItemsRepository::set_status(pool, &item_c.id, "dismissed")
        .await
        .unwrap()
        .unwrap();

    // The regeneration (template switch): reworded A, original-B phrasing, C verbatim,
    // D omitted, E rephrased, plus brand-new F.
    let mut e_rephrased = cand("schedule the beta review with the pilot group next week");
    e_rephrased.due_hint = Some("next week".into());
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2 (different template)",
            None,
            &[
                cand("send the quarterly deck to alice"), // A reworded (0.75)
                cand(b),                                  // B's ORIGINAL phrasing (0.75 vs edit)
                cand(c),                                  // C verbatim
                e_rephrased,                              // E rephrased (7/9 ≈ 0.78)
                cand("collect pilot feedback survey responses"), // F: new
            ],
        )
        .await
    );

    let rows = items(pool, &meeting_id).await;
    assert_eq!(
        rows.len(),
        5,
        "A B C E F — D deleted, zero duplicates anywhere"
    );

    let a_after = rows.iter().find(|r| r.id == item_a.id).expect("A kept");
    assert_eq!(
        (a_after.status.as_str(), a_after.description.as_str()),
        ("completed", a)
    );
    let b_after = rows.iter().find(|r| r.id == item_b.id).expect("B kept");
    assert_eq!(b_after.description, b_edited, "B keeps the user's edit");
    assert!(b_after.user_edited);
    assert!(
        b_after.assignee_is_self,
        "B keeps the user's assignee change"
    );
    let c_after = rows.iter().find(|r| r.id == item_c.id).expect("C kept");
    assert_eq!(c_after.status, "dismissed");
    assert!(
        !rows.iter().any(|r| r.id == item_d.id),
        "pristine D deleted"
    );
    let e_after = rows
        .iter()
        .find(|r| r.id == item_e.id)
        .expect("E kept, same id");
    assert_eq!(
        e_after.description,
        "schedule the beta review with the pilot group next week"
    );
    assert_eq!(e_after.created_at, item_e.created_at);
    assert_eq!(e_after.status, "open");
    assert!(
        rows.iter()
            .any(|r| r.description == "collect pilot feedback survey responses"),
        "new F inserted"
    );
}

/// Dogfood regression (2026-07 incident, real pair texts): regenerating with a DIFFERENT
/// template made the LLM compress every action item into a subset-style rephrasing —
/// "Send the presentation deck to participants" → "Send the deck" (Jaccard 0.50),
/// "Order pizza for the meeting" → "Order the pizza" (0.60) — beyond the old
/// Jaccard-only ≥ 0.7 matcher, so every candidate failed to match its protected row and
/// inserted as a duplicate, DOUBLING the list. The overlap-coefficient matcher
/// (max(Jaccard, |∩|/min) in `diff::similarity`) absorbs these pairs.
///
/// The third incident pair, "Set up the room" (completed) → "Look at the room" (0.33
/// Jaccard / 0.5 overlap), is intentionally below the token-level threshold and is
/// covered by the OTHER layer: the protected row's description rides in the extraction
/// prompt's ALREADY TRACKED section, so the model suppresses the rephrasing before the
/// diff ever sees it — modeled here by its absence from the candidate fixtures (the
/// prompt plumbing is pinned in `extractor.rs` / `mod.rs` unit tests).
#[tokio::test]
async fn template_switch_subset_rephrasings_create_zero_duplicates() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    // v1 extraction, then the user takes ownership of all three rows.
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1 (standard template)",
            None,
            &[
                cand("Send the deck to participants"),
                cand("Order pizza"),
                cand("Set up the room"),
            ],
        )
        .await
    );
    let rows = items(pool, &meeting_id).await;
    let id_of = |text: &str| -> String {
        rows.iter()
            .find(|r| r.description == text)
            .unwrap()
            .id
            .clone()
    };
    let (deck_id, pizza_id, room_id) = (
        id_of("Send the deck to participants"),
        id_of("Order pizza"),
        id_of("Set up the room"),
    );

    // User edits (→ user_edited = 1, protected) — the incident's protected texts.
    let deck_text = "Send the presentation deck to participants";
    let deck = ActionItemsRepository::update_content(
        pool,
        &deck_id,
        deck_text,
        None,
        false,
        None,
        None,
        None,
        &diff::content_key(deck_text),
    )
    .await
    .unwrap()
    .unwrap();
    let pizza_text = "Order pizza for the meeting";
    let pizza = ActionItemsRepository::update_content(
        pool,
        &pizza_id,
        pizza_text,
        None,
        false,
        None,
        None,
        None,
        &diff::content_key(pizza_text),
    )
    .await
    .unwrap()
    .unwrap();
    let room = ActionItemsRepository::set_status(pool, &room_id, "completed")
        .await
        .unwrap()
        .unwrap();

    // The template-switch regeneration: every candidate is a subset-style rephrasing of
    // a protected row. "Look at the room" is absent — suppressed by the prompt layer.
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2 (switched template)",
            None,
            &[cand("Send the deck"), cand("Order the pizza")],
        )
        .await
    );

    let rows = items(pool, &meeting_id).await;
    assert_eq!(
        rows.len(),
        3,
        "zero inserts — the incident doubled this list to 6: {rows:#?}"
    );
    let deck_after = rows
        .iter()
        .find(|r| r.id == deck_id)
        .expect("deck row kept");
    assert_eq!(deck_after.description, deck_text, "user's edit untouched");
    assert!(deck_after.user_edited);
    assert_eq!(deck_after.updated_at, deck.updated_at, "row not rewritten");
    let pizza_after = rows
        .iter()
        .find(|r| r.id == pizza_id)
        .expect("pizza row kept");
    assert_eq!(pizza_after.description, pizza_text, "user's edit untouched");
    assert_eq!(
        pizza_after.updated_at, pizza.updated_at,
        "row not rewritten"
    );
    let room_after = rows
        .iter()
        .find(|r| r.id == room_id)
        .expect("room row kept");
    assert_eq!(room_after.status, "completed");
    assert_eq!(room_after.updated_at, room.updated_at, "row not rewritten");
}

/// Dogfood regression (2026-07 zero-candidate incident, the sticky-protection half):
/// the user completed an item, later UN-checked it (completed → open), and the next
/// regeneration omitted it from the candidates — the un-check had made the row pristine
/// again (`status='open', user_edited=0`) and the diff deleted it as "unmatched
/// pristine". Every user status transition now sets `user_edited = 1`, so the reopened
/// row is protected forever. The re-run here carries NON-zero candidates, so it is the
/// sticky protection — not the zero-candidate valve — that saves the row.
#[tokio::test]
async fn reopened_item_survives_rerun_that_omits_it() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand("confirm the venue booking"), cand("send the deck")],
        )
        .await
    );
    let target = items(pool, &meeting_id)
        .await
        .into_iter()
        .find(|r| r.description == "confirm the venue booking")
        .unwrap();
    assert!(!target.user_edited, "fresh extraction starts pristine");

    // The incident's exact sequence: complete, then un-check back to open.
    let completed = ActionItemsRepository::set_status(pool, &target.id, "completed")
        .await
        .unwrap()
        .unwrap();
    assert!(
        completed.user_edited,
        "completing marks the row user-touched"
    );
    let reopened = ActionItemsRepository::set_status(pool, &target.id, "open")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, "open");
    assert!(
        reopened.user_edited,
        "un-checking must NOT reset protection (the incident's root cause)"
    );

    // Regeneration whose candidates OMIT the reopened item (the over-suppressed
    // extractor dropped it; here another task keeps the candidate list non-empty).
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2",
            None,
            &[cand("send the deck")]
        )
        .await
    );

    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 2, "nothing deleted: {rows:#?}");
    let survivor = rows
        .iter()
        .find(|r| r.id == target.id)
        .expect("reopened row survives (the incident deleted it as unmatched pristine)");
    assert_eq!(survivor.status, "open");
    assert_eq!(survivor.description, "confirm the venue booking");
}

/// Dogfood regression (2026-07 zero-candidate incident, the safety-valve half): the
/// extractor returned ZERO candidates while pristine rows existed — suspect, not truth.
/// The run must apply nothing (zero deletes) and must NOT write the ledger fingerprint,
/// so a later run against the same summary retries instead of being fingerprint-skipped.
#[tokio::test]
async fn zero_candidate_run_with_pristine_rows_applies_nothing_and_keeps_ledger() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v1",
            None,
            &[cand("send the deck"), cand("book the room")],
        )
        .await
    );
    let rows_before = items(pool, &meeting_id).await;
    let ledger_before = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();

    // Regenerated summary (new fingerprint), but the extractor over-suppressed to [].
    assert!(
        !extract_pass(pool, &meeting_id, "summary v2", None, &[]).await,
        "suspect zero-candidate run must apply nothing"
    );

    let rows_after = items(pool, &meeting_id).await;
    assert_eq!(
        format!("{rows_before:?}"),
        format!("{rows_after:?}"),
        "zero deletes, zero churn — the incident wiped a row here"
    );
    let ledger_after = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        format!("{ledger_before:?}"),
        format!("{ledger_after:?}"),
        "ledger untouched — the zero-candidate result is suspect, not truth"
    );
    assert_eq!(
        ledger_after.summary_fingerprint,
        diff::extraction_fingerprint("summary v1", None),
        "fingerprint still the LAST TRUSTED run's"
    );

    // Because the ledger was not written, a later run against the SAME summary v2 is
    // not fingerprint-skipped — a healthy extraction can still repair the state.
    assert!(
        extract_pass(
            pool,
            &meeting_id,
            "summary v2",
            None,
            &[cand("send the deck"), cand("book the room")],
        )
        .await,
        "retry against the same summary must proceed (ledger was not poisoned)"
    );
}

/// The valve's deliberate edge: a summary that genuinely contains no action items with
/// NO pristine rows to protect (none at all, or protected-only) does apply — nothing to
/// delete, and the ledger write preserves idempotence for genuinely empty summaries.
#[tokio::test]
async fn zero_candidate_run_without_pristine_rows_writes_ledger() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let meeting_id = create_meeting(pool).await;

    // No rows at all → ledger written.
    assert!(
        extract_pass(pool, &meeting_id, "summary with no tasks", None, &[]).await,
        "genuinely empty extraction over an empty meeting proceeds"
    );
    assert!(items(pool, &meeting_id).await.is_empty());
    let ledger = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .expect("ledger written for a genuinely empty summary");
    assert_eq!(ledger.item_count, 0);
    assert_eq!(
        ledger.summary_fingerprint,
        diff::extraction_fingerprint("summary with no tasks", None)
    );

    // Idempotence preserved: the identical summary re-run is a fingerprint no-op.
    assert!(
        !extract_pass(pool, &meeting_id, "summary with no tasks", None, &[]).await,
        "identical empty summary short-circuits on the ledger"
    );

    // Protected-only rows don't trip the valve either (nothing pristine to protect):
    // the manual row survives and the ledger advances to the new summary.
    ActionItemsRepository::create(
        pool,
        Some(&meeting_id),
        &cand("water the office plants"),
        "manual",
        &diff::content_key("water the office plants"),
    )
    .await
    .unwrap();
    assert!(extract_pass(pool, &meeting_id, "summary v2, still no tasks", None, &[]).await);
    let rows = items(pool, &meeting_id).await;
    assert_eq!(rows.len(), 1, "manual row untouched");
    assert_eq!(rows[0].source, "manual");
    let ledger = ActionItemsRepository::get_extraction(pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger.summary_fingerprint,
        diff::extraction_fingerprint("summary v2, still no tasks", None),
        "ledger advances when there was nothing pristine to protect"
    );
}
