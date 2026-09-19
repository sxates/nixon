'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, Pencil } from 'lucide-react';
import { cn } from '@/lib/utils';
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

  const displayTitle = title?.trim() || 'Untitled meeting';
  const reelDate = formatReelDate(createdAt);
  const reelTime = formatReelTime(createdAt);
  const reelLength = formatReelLength(durationSeconds ?? durationFromTranscripts(transcripts));
  const reelSource = source?.trim() || null;

  const titleField = isEditing ? (
    <input
      ref={inputRef}
      type="text"
      value={title === PLACEHOLDER_TITLE ? '' : title}
      placeholder="Untitled meeting"
      onChange={(e) => onTitleChange(e.target.value)}
      onBlur={() => void commitTitle()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          void commitTitle();
        } else if (e.key === 'Escape') {
          e.preventDefault();
          cancelEditing();
        }
      }}
      className="w-full rounded-md border border-input bg-muted px-2 py-1 font-display text-[22px] font-semibold tracking-[-0.011em] text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
    />
  ) : (
    <button
      type="button"
      onClick={startEditing}
      title="Click to rename"
      className="group -ml-2 flex w-fit max-w-full items-center gap-2 rounded-md px-2 py-1 text-left transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <h1
        className={cn(
          'truncate font-display text-[22px] font-semibold tracking-[-0.011em]',
          displayTitle === 'Untitled meeting' ? 'text-muted-foreground' : 'text-foreground',
        )}
      >
        {displayTitle}
      </h1>
      <Pencil className="h-4 w-4 flex-shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
    </button>
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

  // Single-column document layout: back button + editable title + the engraved identity
  // line. The back button used to hang in the gutter left of the reading column
  // (`lg:absolute lg:-left-9`) so the title lined up with the participants row and tabs
  // below. specs/0064 W4 moved this header out of that column and across the full width,
  // which left the hang pulling the button 36px into the page's own padding — hard against
  // the sidebar (owner feedback 2026-09-19). It is inline again, inside that padding.
  return (
    <div className="relative flex items-start gap-1.5">
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to meetings"
          title="Back to meetings"
          className="-ml-1 mt-1 inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ArrowLeft className="h-[18px] w-[18px]" />
        </button>
      )}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {titleField}
        {identityLine}
      </div>
    </div>
  );
}
