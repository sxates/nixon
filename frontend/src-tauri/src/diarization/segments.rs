//! Loading a meeting's transcript segments for diarization alignment
//! (specs/0010 P1-B): the thin DB row → [`AlignableSegment`]-shaped view, plus
//! the persisted channel-tag mapping (specs/0029 WS3.4). Split out of
//! `pipeline.rs` under the specs/0042 file-size ratchet (0044 follow-through).

use anyhow::{Context, Result};

use crate::diarization::align::Channel;

/// Transcript segment to attribute: id + recording-relative window + the
/// capture-channel tag recorded at capture time (specs/0029 WS3.4).
pub struct SegmentRow {
    pub id: String,
    pub start: f32,
    pub end: f32,
    pub channel: Channel,
}

/// Map a persisted `transcripts.channel` value to the alignment [`Channel`].
/// Unknown values and NULL (legacy rows, batch sources) map to [`Channel::Mixed`],
/// which reproduces the pre-WS3.4 overlap heuristic exactly.
pub fn channel_from_db(value: Option<&str>) -> Channel {
    match value {
        Some("microphone") => Channel::Microphone,
        Some("system") => Channel::System,
        _ => Channel::Mixed,
    }
}

/// Load a meeting's transcript segments (id + audio window + channel tag),
/// ordered by start. Segments missing both timestamps are skipped (they cannot
/// be aligned).
pub async fn load_segments(pool: &sqlx::SqlitePool, meeting_id: &str) -> Result<Vec<SegmentRow>> {
    let rows = sqlx::query_as::<_, (String, Option<f64>, Option<f64>, Option<String>)>(
        "SELECT id, audio_start_time, audio_end_time, channel
         FROM transcripts
         WHERE meeting_id = ?
         ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("load transcript segments for meeting {meeting_id}"))?;

    Ok(rows
        .into_iter()
        .filter_map(|(id, start, end, channel)| {
            let channel = channel_from_db(channel.as_deref());
            match (start, end) {
                (Some(s), Some(e)) => Some(SegmentRow {
                    id,
                    start: s as f32,
                    end: e as f32,
                    channel,
                }),
                // A row with only a start is still alignable as a zero-length point.
                (Some(s), None) => Some(SegmentRow {
                    id,
                    start: s as f32,
                    end: s as f32,
                    channel,
                }),
                _ => None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_from_db_maps_tags_and_defaults_legacy_to_mixed() {
        assert_eq!(channel_from_db(Some("microphone")), Channel::Microphone);
        assert_eq!(channel_from_db(Some("system")), Channel::System);
        assert_eq!(channel_from_db(Some("mixed")), Channel::Mixed);
        // Legacy rows (NULL) and unexpected values fall back to the pre-WS3.4
        // overlap heuristic (Mixed) — never to a hard mic/system claim.
        assert_eq!(channel_from_db(None), Channel::Mixed);
        assert_eq!(channel_from_db(Some("garbage")), Channel::Mixed);
        assert_eq!(channel_from_db(Some("")), Channel::Mixed);
    }
}
