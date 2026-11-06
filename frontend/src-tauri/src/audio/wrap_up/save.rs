//! The wrap-up's `saving` stage (specs/0083 W2a tasks 5 and 7): turn the drained session
//! into transcript rows through `transcripts::save_session`, or discard an empty session
//! that holds nothing worth keeping.

use std::path::Path;

use sqlx::SqlitePool;

use super::StoppedSession;
use crate::audio::recording_saver::TranscriptSegment as StoredSegment;
use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::transcripts::{save_session, TranscriptSegment};

/// The saver's segments as transcript rows: blank text dropped, ordered by
/// `audio_start_time` then `sequence_id` (the order `TranscriptContext` saved in).
/// A microphone row gets the `local` speaker key, as the live transcript gave it.
pub fn session_segments(stored: &[StoredSegment]) -> Vec<TranscriptSegment> {
    let mut kept: Vec<&StoredSegment> = stored
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    kept.sort_by(|a, b| {
        a.audio_start_time
            .total_cmp(&b.audio_start_time)
            .then(a.sequence_id.cmp(&b.sequence_id))
    });
    kept.into_iter()
        .map(|s| TranscriptSegment {
            id: s.id.clone(),
            text: s.text.clone(),
            timestamp: s.display_time.clone(),
            audio_start_time: Some(s.audio_start_time),
            audio_end_time: Some(s.audio_end_time),
            duration: Some(s.duration),
            speaker: (s.channel.as_deref() == Some("microphone")).then(|| "local".to_string()),
            channel: s.channel.clone(),
            word_timestamps: None,
        })
        .collect()
}

/// The stop body failed: rebuild the session from the snapshot and the folder's
/// `transcripts.json`, skipping the resumed sessions' segments at its head.
pub fn fallback_session(snapshot: &super::SessionSnapshot) -> StoppedSession {
    let segments = snapshot
        .folder_path
        .as_deref()
        .map(|f| crate::audio::recording_recovery::load_folder_transcript_segments(Path::new(f)))
        .unwrap_or_default();
    // `load_folder_transcript_segments` drops blank rows, so skipping by count is only exact
    // when the prior sessions had none; a blank prior row would cost one current segment,
    // never duplicate one.
    let segments = segments
        .into_iter()
        .skip(if snapshot.resumed {
            snapshot.prior_segment_count
        } else {
            0
        })
        .collect();
    StoppedSession {
        folder_path: snapshot.folder_path.clone(),
        meeting_name: snapshot.meeting_name.clone(),
        resumed: snapshot.resumed,
        prior_audio_duration_seconds: snapshot.prior_audio_duration_seconds,
        meeting_id: snapshot.meeting_id.clone(),
        segments,
        drain: None,
    }
}

/// Typed notes (markdown or a non-empty block list) — the JS `canDiscardMeeting` check.
async fn has_typed_notes(pool: &SqlitePool, meeting_id: &str) -> Result<bool, sqlx::Error> {
    let notes = MeetingNotesRepository::get_notes(pool, meeting_id).await?;
    Ok(notes.is_some_and(|n| {
        let md = n.notes_markdown.as_deref().map(str::trim).unwrap_or("");
        let json = n.notes_json.as_deref().map(str::trim).unwrap_or("");
        !md.is_empty() || (!json.is_empty() && json != "[]")
    }))
}

/// May an empty session's meeting be deleted? Only with no typed notes AND nothing durable
/// (`meetings::discard`). Any error keeps the meeting (specs/0019 WS6.1: never destroy data).
pub async fn may_discard(pool: &SqlitePool, meeting_id: &str, folder: Option<&str>) -> bool {
    match has_typed_notes(pool, meeting_id).await {
        Ok(false) => {}
        Ok(true) => return false,
        Err(e) => {
            log::warn!("Wrap-up: couldn't check notes for {meeting_id}; keeping it: {e}");
            return false;
        }
    }
    match crate::meetings::discard::recording_is_safe_to_discard(pool, meeting_id, folder).await {
        Ok(safe) => safe,
        Err(e) => {
            log::warn!("Wrap-up: couldn't check {meeting_id} is safe to discard; keeping it: {e}");
            false
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved { meeting_id: String, title: String },
    Discarded { meeting_id: String },
}

/// Save the session, or discard it when it is empty and safe to (task 7).
pub async fn save_stopped(
    pool: &SqlitePool,
    stopped: &StoppedSession,
) -> anyhow::Result<SaveOutcome> {
    let meeting_id = stopped
        .meeting_id
        .as_deref()
        .filter(|id| !id.trim().is_empty());
    if let (Some(id), true) = (meeting_id, stopped.segments.is_empty()) {
        if may_discard(pool, id, stopped.folder_path.as_deref()).await {
            crate::meetings::delete::delete_meeting_and_folder(
                pool,
                id,
                crate::meetings::delete::DELETE_LEASE_WAIT,
            )
            .await
            .map_err(anyhow::Error::msg)?;
            return Ok(SaveOutcome::Discarded {
                meeting_id: id.to_string(),
            });
        }
    }
    let title = title_for(pool, stopped, meeting_id).await;
    let saved = save_session(
        pool,
        &title,
        &stopped.segments,
        stopped.folder_path.clone(),
        meeting_id,
        stopped.resumed,
        stopped.prior_audio_duration_seconds,
    )
    .await?;
    Ok(SaveOutcome::Saved {
        meeting_id: saved.meeting_id,
        title,
    })
}

/// The recording's name, falling back to the row's title (JS: `savedMeetingName ||
/// meetingTitle || 'New Meeting'`).
async fn title_for(
    pool: &SqlitePool,
    stopped: &StoppedSession,
    meeting_id: Option<&str>,
) -> String {
    if let Some(name) = stopped
        .meeting_name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
    {
        return name.to_string();
    }
    if let Some(id) = meeting_id {
        if let Ok(Some(m)) =
            crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
                pool, id,
            )
            .await
        {
            return m.title;
        }
    }
    "New Meeting".to_string()
}

/// Bookkeeping: the pinned summary language, or the detected one (best-effort).
pub fn apply_summary_language(folder: Option<&str>, pinned: Option<&str>, texts: &[String]) {
    let Some(folder) = folder.map(Path::new) else {
        return;
    };
    let result = match pinned {
        Some(lang) => {
            crate::summary::metadata::write_summary_language_to_metadata(folder, Some(lang))
        }
        None => match crate::summary::language_detection::detect_summary_language(texts).language {
            Some(lang) => crate::summary::metadata::write_detected_summary_language_to_metadata(
                folder,
                Some(&lang),
            ),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        log::warn!("Wrap-up: couldn't record the summary language: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(seq: u64, start: f64, text: &str, channel: Option<&str>) -> StoredSegment {
        StoredSegment {
            id: format!("seg_{seq}"),
            text: text.into(),
            audio_start_time: start,
            audio_end_time: start + 1.0,
            duration: 1.0,
            display_time: format!("[00:0{seq}]"),
            confidence: 0.9,
            sequence_id: seq,
            channel: channel.map(str::to_string),
        }
    }

    #[test]
    fn segments_drop_blanks_and_sort_by_start_then_sequence() {
        let out = session_segments(&[
            stored(3, 2.0, "third", Some("system")),
            stored(1, 0.0, "  ", None),
            stored(2, 1.0, "second", Some("microphone")),
            stored(4, 1.0, "tie later seq", None),
        ]);
        let texts: Vec<_> = out.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["second", "tie later seq", "third"]);
        assert_eq!(out[0].timestamp, "[00:02]");
        assert_eq!(out[0].audio_start_time, Some(1.0));
        assert_eq!(out[0].speaker.as_deref(), Some("local"));
        assert_eq!(out[0].channel.as_deref(), Some("microphone"));
        assert_eq!(out[2].speaker, None);
    }
}
