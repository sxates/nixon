//! The wrap-up stages, in order (specs/0083 W2a): stopping/draining → saving → bookkeeping
//! (summary language, defer marker, backlog handoff) → diarizing → summarizing → done.
//! Nothing here waits on the webview except the bounded handoff answer.

use std::future::Future;
use std::time::Duration;

use sqlx::SqlitePool;
use tauri::{Manager, Runtime};

use super::auto_summary::{self, GateInput, SkipReason};
use super::save::{self, SaveOutcome};
use super::stop_action::{self, HandoffOutcome, StopAction, Toast};
use super::{Reporter, SessionSnapshot, Stage, StoppedSession};
use crate::audio::processing_mode::MODE_DEFER;
use crate::audio::transcript_sink::DrainOutcome;
use crate::database::repositories::meeting::MeetingsRepository;

pub(super) struct Inputs {
    pub snapshot: SessionSnapshot,
    /// `(started_deferred, ever_deferred)` from `live_toggle`, read at the stop.
    pub deferral: (bool, bool),
    pub handoff_wait: Duration,
}

const INCOMPLETE_TOAST: Toast = Toast {
    title: "Transcription didn\u{2019}t fully finish",
    description: Some(
        "We saved the transcript captured so far \u{2014} the end of the meeting may be incomplete.",
    ),
};

pub(super) async fn run<R: Runtime, F>(mut rep: Reporter<R>, inputs: Inputs, stop: F)
where
    F: Future<Output = Result<StoppedSession, String>>,
{
    let stopped = match stop.await {
        Ok(stopped) => stopped,
        Err(e) => {
            log::warn!("Wrap-up: the stop failed ({e}); saving what was captured");
            save::fallback_session(&inputs.snapshot)
        }
    };
    let incomplete = stopped.drain == Some(DrainOutcome::TimedOut);
    rep.stage(Stage::Saving, |s| {
        s.transcription_incomplete = incomplete;
        if s.meeting_id.is_none() {
            s.meeting_id = stopped.meeting_id.clone();
        }
    });

    let pool = rep
        .app
        .try_state::<crate::state::AppState>()
        .map(|s| s.db_manager.pool().clone());
    let Some(pool) = pool else {
        return rep.fail(
            Stage::Saving,
            "The database isn't ready, so the meeting wasn't saved.".into(),
        );
    };
    let meeting_id = match save::save_stopped(&pool, &stopped).await {
        Ok(SaveOutcome::Discarded { meeting_id }) => {
            rep.stage(Stage::Discarded, |s| s.meeting_id = Some(meeting_id));
            return rep.stage(Stage::Done, |_| {});
        }
        Ok(SaveOutcome::Saved { meeting_id, title }) => {
            let toast = (incomplete && !stopped.segments.is_empty()).then_some(INCOMPLETE_TOAST);
            rep.stage(Stage::Saved, |s| {
                s.meeting_id = Some(meeting_id.clone());
                s.title = Some(title);
                s.toast = toast;
            });
            meeting_id
        }
        Err(e) => return rep.fail(Stage::Saving, format!("Couldn't save the meeting: {e:#}")),
    };

    // Bookkeeping: best-effort, never un-saves anything.
    let texts: Vec<String> = stopped.segments.iter().map(|s| s.text.clone()).collect();
    save::apply_summary_language(
        stopped.folder_path.as_deref(),
        super::prefs::summary_language_default().as_deref(),
        &texts,
    );
    let (action, handoff) = processing_mode(&mut rep, &pool, &meeting_id, &inputs).await;
    let follow_up = stop_action::stop_follow_up(action, handoff);

    if follow_up.auto_diarize {
        if !still_exists(&pool, &meeting_id).await {
            return rep.stage(Stage::Done, |_| {});
        }
        rep.stage(Stage::Diarizing, |s| s.toast = follow_up.toast.clone());
        // Starts the pass (if it applies) and returns; the pass reports through
        // `on_diarization_outcome`. The summary runs alongside it (owner, 2026-11-05).
        crate::audio::lifecycle::finish_processing(&rep.app, &meeting_id, false).await;
    }

    if !still_exists(&pool, &meeting_id).await {
        return rep.stage(Stage::Done, |_| {});
    }
    let in_backlog = handoff == Some(HandoffOutcome::Accepted);
    match gate(&pool, &meeting_id, in_backlog).await {
        Some(reason) => rep.stage(Stage::SummarySkipped, |s| {
            s.reason = Some(reason);
            s.toast = auto_summary::skip_toast(reason);
        }),
        None => match crate::summary::commands::start_summary_generation_for_meeting(
            rep.app.clone(),
            pool.clone(),
            meeting_id.clone(),
        )
        .await
        {
            Ok(_) => rep.stage(Stage::Summarizing, |_| {}),
            Err(e) => rep.stage(Stage::SummarySkipped, |s| {
                s.reason = Some(SkipReason::StartFailed);
                s.toast = auto_summary::skip_toast(SkipReason::StartFailed);
                s.message = Some(e);
            }),
        },
    }
    rep.stage(Stage::Done, |_| {});
}

/// Defer marker first (spec 0051 WS2), then the `process-now` handoff.
async fn processing_mode<R: Runtime>(
    rep: &mut Reporter<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    inputs: &Inputs,
) -> (StopAction, Option<HandoffOutcome>) {
    let override_mode = match MeetingsRepository::get_processing_mode(pool, meeting_id).await {
        Ok(mode) => mode,
        Err(e) => {
            log::warn!("Wrap-up: couldn't read the processing mode (meeting saved fine): {e}");
            return (StopAction::None, None);
        }
    };
    let (started, ever) = inputs.deferral;
    let action = stop_action::stop_action(started, ever, override_mode.as_deref());
    if action == StopAction::None {
        return (action, None);
    }
    if let Err(e) =
        MeetingsRepository::set_processing_mode(pool, meeting_id, Some(MODE_DEFER)).await
    {
        log::warn!("Wrap-up: couldn't write the defer marker for {meeting_id}: {e}");
    }
    if action != StopAction::ProcessNow {
        return (action, None);
    }
    let rx = super::handoff::register(meeting_id);
    rep.stage(Stage::Handoff, |_| {});
    let outcome = super::handoff::wait(meeting_id, rx, inputs.handoff_wait).await;
    log::info!("Wrap-up: backlog handoff for {meeting_id}: {outcome:?}");
    (action, Some(outcome))
}

async fn still_exists(pool: &SqlitePool, meeting_id: &str) -> bool {
    !matches!(
        MeetingsRepository::get_meeting_metadata(pool, meeting_id).await,
        Ok(None)
    )
}

/// Read the gate's inputs; a read failure skips with `model-check-failed`.
async fn gate(pool: &SqlitePool, meeting_id: &str, in_backlog: bool) -> Option<SkipReason> {
    match gate_input(pool, meeting_id, in_backlog).await {
        Ok(input) => auto_summary::skip_reason(&input),
        Err(e) => {
            log::warn!("Wrap-up: couldn't read the auto-summary inputs for {meeting_id}: {e}");
            Some(SkipReason::ModelCheckFailed)
        }
    }
}

async fn gate_input(
    pool: &SqlitePool,
    meeting_id: &str,
    in_backlog: bool,
) -> Result<GateInput, sqlx::Error> {
    use crate::database::repositories::{
        meeting_note::MeetingNotesRepository, setting::SettingsRepository,
        summary::SummaryProcessesRepository,
    };
    let transcript_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await?;
    // The summary grounds on markdown notes alone (`start_summary_generation_for_meeting`).
    let has_notes = MeetingNotesRepository::get_notes(pool, meeting_id)
        .await?
        .and_then(|n| n.notes_markdown)
        .is_some_and(|m| !m.trim().is_empty());
    Ok(GateInput {
        has_summary: SummaryProcessesRepository::get_summary_data(pool, meeting_id)
            .await?
            .is_some(),
        processing_mode: MeetingsRepository::get_processing_mode(pool, meeting_id).await?,
        in_backlog,
        enabled: super::prefs::auto_summary(),
        transcript_count,
        has_notes,
        has_model: SettingsRepository::get_model_config(pool)
            .await?
            .is_some_and(|c| !c.model.trim().is_empty()),
    })
}
