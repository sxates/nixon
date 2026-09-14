//! Tauri commands for the People entity (specs/0016 Phase 1b).
//!
//! Frontend → Rust surface for the People directory and speaker→person association:
//! - [`api_list_people`]                      — all people, name-ordered.
//! - [`api_list_people_ranked`]               — all people, ranked (starred + frequency).
//! - [`api_set_person_starred`]               — pin/unpin a person for ranked pick-lists.
//! - [`api_get_person`]                       — one person by id (the person-detail page).
//! - [`api_create_person`]                    — create (upsert-by-email).
//! - [`api_update_person`]                    — edit name/email/role/notes.
//! - [`api_delete_person`]                    — "forget this person" (detaches speakers).
//! - [`api_set_person_voiceprint_opt_out`]    — flip the ADR-0007 §2 per-person flag.
//! - [`api_assign_speaker_to_person`]         — link a detected speaker to a person.
//!
//! Owner identity (specs/0018): "which addresses are me" + "claim a participant as me":
//! - [`api_get_owner_emails`]                 — list the owner's declared addresses.
//! - [`api_add_owner_email`]                  — add one (runs the backfill sweep).
//! - [`api_remove_owner_email`]               — remove one (no un-merge).
//! - [`api_claim_participant_as_me`]          — "This is me": record email + merge into owner.
//!
//! All return user-actionable error strings; the DB I/O lives in `PeopleRepository`.

use tauri::{AppHandle, Manager, Runtime};

use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
use crate::database::repositories::owner_emails::OwnerEmailsRepository;
use crate::database::repositories::people::{PeopleRepository, Person};
use crate::people::enroll::OWNER_PERSON_ID;
use crate::people::merge::merge_person_into_owner;
use crate::state::AppState;

/// All people, ordered by display name — the People directory list.
#[tauri::command]
pub async fn api_list_people<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Person>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    PeopleRepository::list(pool)
        .await
        .map_err(|e| format!("Failed to load people: {e}"))
}

/// All people, ranked for pick-lists (specs/0038 WS5.a): starred first, then by call
/// frequency (most-seen first), then alphabetically. Returns the same `Person` DTO as
/// [`api_list_people`], now carrying `starred`, so a user's top collaborators float to the
/// top of the attendee/speaker pickers instead of an unranked alphabetical wall.
#[tauri::command]
pub async fn api_list_people_ranked<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Person>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    PeopleRepository::list_ranked(pool)
        .await
        .map_err(|e| format!("Failed to load people: {e}"))
}

/// Star/unstar a person (specs/0038 WS5.a) so they pin to the top of ranked pick-lists.
#[tauri::command]
pub async fn api_set_person_starred<R: Runtime>(
    app: AppHandle<R>,
    person_id: String,
    starred: bool,
) -> Result<(), String> {
    if person_id.trim().is_empty() {
        return Err("a person must be selected".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let updated = PeopleRepository::set_starred(pool, &person_id, starred)
        .await
        .map_err(|e| format!("Failed to update star: {e}"))?;
    if !updated {
        return Err("That person no longer exists".to_string());
    }
    Ok(())
}

/// Create a new person. Upsert-by-email: if `email` is provided and already belongs to a
/// person, that existing person is returned instead of erroring (the caller gets a usable
/// person back either way). `displayName` is required; the rest are optional.
#[tauri::command]
pub async fn api_create_person<R: Runtime>(
    app: AppHandle<R>,
    display_name: String,
    email: Option<String>,
    role: Option<String>,
    notes: Option<String>,
) -> Result<Person, String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err("A name is required to create a person".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    PeopleRepository::create(
        pool,
        display_name,
        email.as_deref(),
        role.as_deref(),
        notes.as_deref(),
    )
    .await
    .map_err(|e| format!("Failed to create person: {e}"))
}

/// One person by id, or `None` if they've been forgotten — backs the dedicated
/// person-detail page (specs/0038 dogfood feedback #3).
#[tauri::command]
pub async fn api_get_person<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<Option<Person>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    PeopleRepository::get(pool, &id)
        .await
        .map_err(|e| format!("Failed to load person: {e}"))
}

/// Update a person's editable fields. Setting an `email` that already belongs to a
/// different person is rejected with an actionable message.
#[tauri::command]
pub async fn api_update_person<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    display_name: String,
    email: Option<String>,
    role: Option<String>,
    notes: Option<String>,
) -> Result<(), String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err("A name is required".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let updated = PeopleRepository::update(
        pool,
        &id,
        display_name,
        email.as_deref(),
        role.as_deref(),
        notes.as_deref(),
    )
    .await
    .map_err(|e| format!("Failed to update person: {e}"))?;
    if !updated {
        return Err("That person no longer exists".to_string());
    }
    Ok(())
}

/// "Forget this person": delete the person and detach it from every speaker. Phase 1c
/// will also cascade-delete their voiceprints in the same transaction.
#[tauri::command]
pub async fn api_delete_person<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let deleted = PeopleRepository::delete(pool, &id)
        .await
        .map_err(|e| format!("Failed to forget person: {e}"))?;
    if !deleted {
        return Err("That person no longer exists".to_string());
    }
    Ok(())
}

/// Flip the per-person "don't store this person's voice" flag (ADR-0007 §2). Identity
/// association still works when this is on — it only blocks voice modeling (wired in 1c).
#[tauri::command]
pub async fn api_set_person_voiceprint_opt_out<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    opt_out: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let updated = PeopleRepository::set_voiceprint_opt_out(pool, &id, opt_out)
        .await
        .map_err(|e| format!("Failed to update voiceprint setting: {e}"))?;
    if !updated {
        return Err("That person no longer exists".to_string());
    }
    Ok(())
}

/// Associate a detected speaker in a transcript with a durable person (specs/0016 1b):
/// sets `speakers.person_id` and copies the person's name/email onto the speaker row.
/// Succeeds regardless of the person's `voiceprint_opt_out` flag (identity, not voice).
#[tauri::command]
pub async fn api_assign_speaker_to_person<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_key: String,
    person_id: String,
) -> Result<(), String> {
    if meeting_id.trim().is_empty() || speaker_key.trim().is_empty() {
        return Err("meeting and speaker are required".to_string());
    }
    if person_id.trim().is_empty() {
        return Err("a person must be selected".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let assigned =
        PeopleRepository::assign_speaker_to_person(pool, &meeting_id, &speaker_key, &person_id)
            .await
            .map_err(|e| format!("Failed to assign speaker to person: {e}"))?;
    if !assigned {
        return Err(
            "Couldn't link that speaker — the person or speaker no longer exists".to_string(),
        );
    }

    // specs/0038 WS6.c: identifying a speaker also puts that person on the meeting's
    // participant roster, so a named speaker always shows up as a participant. The owner
    // ("You") is implicit and never a roster row (specs/0018) — `add_identified` skips the
    // singleton owner person and dedupes on the `(meeting_id, person_id)` PK, so re-assigning
    // the same speaker never duplicates the row. Best-effort: the identity link is already
    // committed — a roster-add failure (or the owner skip) must not fail the command.
    if let Err(e) =
        MeetingParticipantsRepository::add_identified(pool, &meeting_id, &person_id).await
    {
        log::warn!(
            "speaker-assign: roster add for person {person_id} in meeting {meeting_id} failed (continuing): {e}"
        );
    }

    // Enroll-on-confirm (specs/0016 1c, ADR-0007 §2/§3): opportunistically add this
    // cluster's voiceprint to the person's gallery, behind the consent gate. Best-effort
    // — the identity is already linked; a gating no-op or an enroll error must not fail
    // the command. (Owner/local rows enroll under the singleton "You" person.)
    match crate::people::enroll::enroll_voiceprint_for_speaker(
        pool,
        &meeting_id,
        &speaker_key,
        &person_id,
        // An explicit human "this speaker is that person" — highest attribution trust
        // (specs/0039 WS3). The WS3 cluster-quality guards still apply inside.
        crate::people::enroll::EnrollConfidence::UserConfirmed,
    )
    .await
    {
        Ok(true) => log::info!(
            "enrolled voiceprint for {speaker_key} in meeting {meeting_id} (person {person_id})"
        ),
        Ok(false) => {} // correctly gated off, or nothing to enroll — expected.
        Err(e) => log::warn!(
            "voiceprint enroll for {speaker_key} in meeting {meeting_id} failed (continuing): {e:#}"
        ),
    }

    // specs/0044 WS3: the speaker now resolves to a real person name — debounced
    // refresh of a pristine summary whose name set is stale. Fire-and-forget.
    crate::summary::refresh::schedule_name_refresh(&app, pool.clone(), &meeting_id);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Owner identity — "which addresses are me" + "claim a participant as me" (specs/0018).
// ---------------------------------------------------------------------------------------

/// The owner's declared invite addresses (normalized, ordered) — the Settings list.
#[tauri::command]
pub async fn api_get_owner_emails<R: Runtime>(app: AppHandle<R>) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    OwnerEmailsRepository::list(pool)
        .await
        .map_err(|e| format!("Failed to load your email addresses: {e}"))
}

/// Add one owner address (normalized lower/trim). Runs the backfill sweep — any
/// already-rostered person with this email is folded into the owner. Returns the updated
/// list so the UI can refresh from the source of truth.
#[tauri::command]
pub async fn api_add_owner_email<R: Runtime>(
    app: AppHandle<R>,
    email: String,
) -> Result<Vec<String>, String> {
    if email.trim().is_empty() {
        return Err("Enter an email address".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    OwnerEmailsRepository::add(pool, &email)
        .await
        .map_err(|e| format!("Couldn't add that email address: {e}"))?;
    OwnerEmailsRepository::list(pool)
        .await
        .map_err(|e| format!("Failed to reload your email addresses: {e}"))
}

/// Remove one owner address (the address-level inverse). Stops FUTURE auto-matching; does
/// NOT un-merge people already folded into the owner. Returns the updated list.
#[tauri::command]
pub async fn api_remove_owner_email<R: Runtime>(
    app: AppHandle<R>,
    email: String,
) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    OwnerEmailsRepository::remove(pool, &email)
        .await
        .map_err(|e| format!("Couldn't remove that email address: {e}"))?;
    OwnerEmailsRepository::list(pool)
        .await
        .map_err(|e| format!("Failed to reload your email addresses: {e}"))
}

/// "This is me": claim a meeting participant as the device owner (specs/0018). Idempotent.
///
/// 1. If `personId` is already the owner, no-op.
/// 2. If the person has an email, record it as an owner address (`OwnerEmailsRepository::add`,
///    whose internal sweep is harmless/idempotent here) so future calendar seeds auto-match.
///    An email-less person still merges — claiming works — but can't be matched by future
///    invites (logged; Settings is the email path).
/// 3. Merge the person into the owner: re-point speakers/voiceprints, drop their roster rows
///    app-wide, delete the empty person.
///
/// `meetingId` is accepted for symmetry/telemetry; the merge is app-wide (claiming is an
/// identity statement, not per-meeting).
#[tauri::command]
pub async fn api_claim_participant_as_me<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    person_id: String,
) -> Result<(), String> {
    if person_id.trim().is_empty() {
        return Err("a participant must be selected".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    // Already me → nothing to do.
    if person_id == OWNER_PERSON_ID {
        return Ok(());
    }

    let person = PeopleRepository::get(pool, &person_id)
        .await
        .map_err(|e| format!("Couldn't load that participant: {e}"))?
        .ok_or_else(|| "That participant no longer exists".to_string())?;

    // Record the email as "me" (so future invites auto-match). Email-less → just merge.
    match person
        .email
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        Some(email) => {
            OwnerEmailsRepository::add(pool, email)
                .await
                .map_err(|e| format!("Couldn't record your email address: {e}"))?;
        }
        None => {
            log::info!(
                "claim-as-me: participant {person_id} (meeting {meeting_id}) has no email; \
                 merging into owner but future calendar invites can't auto-match"
            );
        }
    }

    merge_person_into_owner(pool, &person_id)
        .await
        .map_err(|e| format!("Couldn't claim that participant as you: {e}"))
}
