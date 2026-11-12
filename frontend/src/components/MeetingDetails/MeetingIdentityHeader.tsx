'use client';

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from '@/components/ui/page-header';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { formatReelNumber } from '@/lib/reel-number';

interface MeetingIdentityHeaderProps {
  meetingId: string;
  /** Controlled title value (lives in useMeetingData). */
  title: string;
  /** Mirror edits back into the parent's draft title state. */
  onTitleChange: (title: string) => void;
  /** Persist the current title (typically useMeetingData.handleSaveMeetingTitle). */
  onSaveTitle: () => Promise<boolean> | Promise<void> | void;
  /** ISO-8601 (RFC 3339) UTC timestamp the meeting was created. */
  createdAt?: string | null;
  /**
   * Loaded transcripts for this meeting. Used as a duration fallback (max
   * `audio_end_time`) until the authoritative `durationSeconds` from
   * `api_get_meetings` is available.
   */
  transcripts?: Array<{ audio_end_time?: number | null }>;
  /**
   * Archival reel ordinal (specs/0057) from `api_get_meeting_metadata`. Drives the
   * `REEL 0412` handle on the identity line and the reel label. Absent/null (scheduled
   * placeholders, older DTOs) prints the blank `REEL ——` handle.
   */
  reelNumber?: number | null;
  /** Where the recording came from, e.g. "Zoom" / "Imported". Omitted when unknown. */
  source?: string | null;
  /** Distinct speakers on the reel (specs/0057) — the reel label's VOICES line. */
  voices?: number;
  /** Back handler for the inline back button (e.g. router.push('/')). */
  onBack?: () => void;
  /** Right-hand cluster: the scheduled record control and the meeting's overflow menu. */
  actions?: ReactNode;
  className?: string;
}

const PLACEHOLDER_TITLE = '+ New Call';

/** "Tue Jun 24" — local date, no year (the reel number carries the archival ordering). */
function formatReelDate(createdAt?: string | null): string | null {
  if (!createdAt) return null;
  const d = new Date(createdAt);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
}

/** "10:00" — local start time on a 24h clock, so labels stay the same width. */
function formatReelTime(createdAt?: string | null): string | null {
  if (!createdAt) return null;
  const d = new Date(createdAt);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
}

/** durationSeconds → tape-counter "00:42:18". Returns null when absent/zero. */
function formatReelLength(seconds?: number | null): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const total = Math.round(seconds);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(Math.floor(total / 3600))}:${pad(Math.floor(total / 60) % 60)}:${pad(total % 60)}`;
}

/** Best-effort duration from already-loaded transcripts (max audio_end_time). */
function durationFromTranscripts(
  transcripts?: Array<{ audio_end_time?: number | null }>,
): number | null {
  if (!transcripts?.length) return null;
  let max = 0;
  for (const t of transcripts) {
    const end = t.audio_end_time;
    if (typeof end === 'number' && Number.isFinite(end) && end > max) max = end;
  }
  return max > 0 ? max : null;
}

export function MeetingIdentityHeader({
  meetingId,
  title,
  onTitleChange,
  onSaveTitle,
  createdAt,
  transcripts,
  reelNumber,
  source,
  voices,
  onBack,
  actions,
  className,
}: MeetingIdentityHeaderProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  const [isEditing, setIsEditing] = useState(false);
  // Snapshot of the title when editing begins, so Escape can revert cleanly.
  const editStartTitleRef = useRef<string>(title);

  // Authoritative duration (max transcript audio_end_time) from api_get_meetings.
  // Fetched once per meeting; falls back to loaded transcripts in the meantime.
  const [durationSeconds, setDurationSeconds] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDurationSeconds(null);
    if (!meetingId) return;

    const loadDuration = async () => {
      try {
        const meetings = (await invoke('api_get_meetings', { authToken: null })) as Array<{
          id: string;
          durationSeconds?: number;
        }>;
        if (cancelled) return;
        const match = Array.isArray(meetings)
          ? meetings.find((m) => m.id === meetingId)
          : undefined;
        if (match?.durationSeconds != null) setDurationSeconds(match.durationSeconds);
      } catch (err) {
        // Non-fatal: header still renders title + date; duration just stays hidden
        // (or uses the transcript-derived fallback).
        console.warn('Could not load meeting duration:', err);
      }
    };

    loadDuration();
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  const startEditing = useCallback(() => {
    editStartTitleRef.current = title;
    setIsEditing(true);
  }, [title]);

  const commitTitle = useCallback(async () => {
    setIsEditing(false);
    const trimmed = title.trim();
    // Don't persist an empty or placeholder title.
    if (!trimmed || trimmed === PLACEHOLDER_TITLE) {
      onTitleChange(editStartTitleRef.current);
      return;
    }
    if (trimmed !== editStartTitleRef.current.trim()) {
      await onSaveTitle();
    }
  }, [title, onTitleChange, onSaveTitle]);

  const cancelEditing = useCallback(() => {
    onTitleChange(editStartTitleRef.current);
    setIsEditing(false);
  }, [onTitleChange]);

  useEffect(() => {
    if (isEditing) {
      // Focus + select on next tick so the input is mounted.
      const id = window.setTimeout(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      }, 0);
      return () => window.clearTimeout(id);
    }
  }, [isEditing]);

  // The context's unnamed-session placeholder is not a title: show the muted placeholder.
  const realTitle = title === PLACEHOLDER_TITLE ? '' : title;
  const reelDate = formatReelDate(createdAt);
  const reelTime = formatReelTime(createdAt);
  const reelLength = formatReelLength(durationSeconds ?? durationFromTranscripts(transcripts));
  const reelSource = source?.trim() || null;

  const titleField = (
    <MeetingTitleField
      title={realTitle}
      placeholder="Untitled meeting"
      isEditing={isEditing}
      draft={realTitle}
      onDraftChange={onTitleChange}
      onStartEditing={startEditing}
      onCommit={() => void commitTitle()}
      onCancel={cancelEditing}
      inputRef={inputRef}
    />
  );

  // Engraved identity line (specs/0057): the reel handle plus the recording's facts,
  // silkscreened in small caps — "REEL 0412 · TUE JUN 24 · 10:00 · 00:42:18 · ZOOM · 4 VOICES".
  // Unknown segments (no duration yet, no known source, not yet diarized) are simply
  // dropped. This line is the meeting's whole identity: the typed reel-label card that used
  // to duplicate it on the right was removed on 0.1.0 canvas feedback (too tall, redundant).
  const identitySegments = [
    formatReelNumber(reelNumber),
    reelDate,
    reelTime,
    reelLength,
    reelSource,
    voices == null ? null : `${voices} ${voices === 1 ? 'voice' : 'voices'}`,
  ].filter((segment): segment is string => !!segment);

  const identityLine = (
    <p data-testid="meeting-identity-line" className="u-section-label pl-0.5">
      {identitySegments.join(' · ')}
    </p>
  );

  // The one page header (docs/design-system.md): back · editable title · the engraved
  // identity line as the subtitle · the per-meeting actions. The recorder renders the same
  // `PageHeader` + `MeetingTitleField`, so the two screens share one title row. The back
  // button is inline, inside the page padding — it used to hang 36px into it, hard against
  // the sidebar (owner feedback 2026-09-19). The caller owns the outer padding.
  return (
    <PageHeader
      className={className}
      back={onBack ? { label: 'Back to meetings', onClick: onBack } : undefined}
      title={titleField}
      subtitle={identityLine}
      actions={actions}
    />
  );
}
