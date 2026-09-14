'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AlertTriangle, Loader2, RotateCcw, ShieldOff, Trash2, Waves } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { formatMeetingDate } from '@/lib/format-date';
import { cn } from '@/lib/utils';
import type { MeetingMetadata, VoiceprintSampleDto } from '@/types';

interface VoiceprintSamplesListProps {
  /** Person whose voice samples to manage. */
  personId: string;
  /** Display name for toasts. */
  personName?: string;
  /**
   * Cap the list at 52vh and scroll it natively (the dialog host). Unset, the list flows
   * in its host's own scroll container — the person page tab (specs/0056 W5).
   */
  bounded?: boolean;
}

/** Format the 0..1 sample-quality score as a rough percentage, or null when absent. */
function formatQuality(q: number | null): string | null {
  if (q === null || Number.isNaN(q)) return null;
  const pct = Math.round(Math.max(0, Math.min(1, q)) * 100);
  return `${pct}% quality`;
}

/**
 * Per-sample voice gallery management for one person (specs/0039 WS3, task 9), extracted from
 * `VoiceprintSamplesDialog` (specs/0038 dogfood feedback #3) so it can be reused by BOTH the
 * dialog and the person-detail page's "Voice Samples" tab.
 *
 * Lists a person's stored voice samples (`api_list_person_voiceprints` — metadata/ids only,
 * never embeddings) with their source meeting, capture date, quality, and a live/quarantined
 * badge. Each sample can be:
 *   - **Quarantined** (default, recoverable) — soft-deletes it so it stops polluting the
 *     person's centroid/matching, but keeps provenance so it can be restored.
 *   - **Restored** — un-quarantines a previously quarantined sample.
 *   - **Deleted** — PERMANENT; visually distinct + confirm-gated inline ("delete forever?").
 *
 * The quarantine/delete split is the whole point: a wrong in-meeting attribution can poison a
 * voiceprint, and the user needs a recoverable purge (quarantine) separate from an irreversible
 * one (delete). Legacy samples (`sourceSpeakerKey: null`) predate the WS2 span-correction
 * back-link — they can't be auto-retracted, but are fully quarantine/delete-able here, so we
 * never hide them. Global controls (opt-out, "clear all") stay in RecordingSettings.
 *
 * Loads on mount and whenever `personId` changes, so both hosts (the dialog remounts its content
 * on open; the tab mounts when selected) refetch a fresh gallery every time.
 *
 * Scrolling (specs/0056 W5): the dialog passes `bounded` and gets a `max-h-[52vh]
 * overflow-y-auto` native scroller. The person page passes nothing, so a long gallery is
 * scrolled by the page's own `overflow-y-auto` container exactly like the Summary and Recent
 * meetings tabs. (A Radix `ScrollArea` with only a max-height never overflowed — its viewport
 * resolved to `height: auto` — so the tab clipped at 52vh and could not scroll.)
 */
export function VoiceprintSamplesList({
  personId,
  personName,
  bounded = false,
}: VoiceprintSamplesListProps) {
  const [samples, setSamples] = useState<VoiceprintSampleDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Meeting id → title, resolved best-effort so a sample can name its source meeting.
  const [meetingTitles, setMeetingTitles] = useState<Record<string, string>>({});
  // Per-sample in-flight guard (id → true) so a row's buttons disable while its IPC runs.
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  // Sample id currently in the "delete forever?" confirm state (only one at a time).
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);

  const label = personName?.trim() || 'this person';

  const load = useCallback(async () => {
    setError(null);
    setSamples(null);
    try {
      const result = await invoke<VoiceprintSampleDto[]>('api_list_person_voiceprints', {
        personId,
      });
      const list = Array.isArray(result) ? result : [];
      setSamples(list);

      // Resolve source-meeting titles best-effort (one metadata fetch per unique id).
      const ids = Array.from(
        new Set(list.map((s) => s.sourceMeetingId).filter((id): id is string => !!id)),
      );
      if (ids.length > 0) {
        const entries = await Promise.all(
          ids.map(async (id) => {
            try {
              const meta = await invoke<MeetingMetadata>('api_get_meeting_metadata', {
                meetingId: id,
              });
              return [id, meta?.title?.trim() || ''] as const;
            } catch {
              return [id, ''] as const;
            }
          }),
        );
        setMeetingTitles(Object.fromEntries(entries.filter(([, t]) => t)));
      }
    } catch (err) {
      console.error('Failed to load voice samples:', err);
      setError('Could not load voice samples.');
    }
  }, [personId]);

  // (Re)load whenever the person changes; reset transient state alongside.
  useEffect(() => {
    setConfirmingDelete(null);
    setBusy({});
    void load();
  }, [load]);

  const setSampleBusy = useCallback((id: string, value: boolean) => {
    setBusy((prev) => {
      if (!value) {
        const next = { ...prev };
        delete next[id];
        return next;
      }
      return { ...prev, [id]: true };
    });
  }, []);

  // Quarantine (recoverable) and Restore are mirror images — one optimistic flip of the
  // `quarantined` flag, one command, revert on failure — so they share this body. `next`
  // is the target state: true = quarantine, false = restore.
  const toggleQuarantine = useCallback(
    async (sample: VoiceprintSampleDto, next: boolean) => {
      if (busy[sample.id]) return;
      setSampleBusy(sample.id, true);
      setSamples((prev) =>
        prev?.map((s) => (s.id === sample.id ? { ...s, quarantined: next } : s)) ?? prev,
      );
      try {
        await invoke(
          next ? 'api_quarantine_voiceprint_sample' : 'api_restore_voiceprint_sample',
          { sampleId: sample.id },
        );
        toast.success(next ? 'Voice sample quarantined' : 'Voice sample restored', {
          description: next
            ? "It won't affect voice matching. You can restore it here."
            : "It's back in the mix for voice matching.",
        });
      } catch (err) {
        console.error(
          next ? 'Failed to quarantine voice sample:' : 'Failed to restore voice sample:',
          err,
        );
        // Revert the optimistic flip.
        setSamples((prev) =>
          prev?.map((s) => (s.id === sample.id ? { ...s, quarantined: !next } : s)) ?? prev,
        );
        toast.error(next ? 'Could not quarantine this sample.' : 'Could not restore this sample.', {
          description: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSampleBusy(sample.id, false);
      }
    },
    [busy, setSampleBusy],
  );

  // Delete — PERMANENT. Confirm-gated by `confirmingDelete`; removes the row on success.
  const handleDelete = useCallback(
    async (sample: VoiceprintSampleDto) => {
      if (busy[sample.id]) return;
      setSampleBusy(sample.id, true);
      try {
        await invoke('api_delete_voiceprint_sample', { sampleId: sample.id });
        setSamples((prev) => prev?.filter((s) => s.id !== sample.id) ?? prev);
        setConfirmingDelete(null);
        toast.success('Voice sample deleted permanently');
      } catch (err) {
        console.error('Failed to delete voice sample:', err);
        toast.error('Could not delete this sample.', {
          description: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSampleBusy(sample.id, false);
      }
    },
    [busy, setSampleBusy],
  );

  const { activeCount, quarantinedCount } = useMemo(() => {
    const list = samples ?? [];
    const quarantined = list.filter((s) => s.quarantined).length;
    return { activeCount: list.length - quarantined, quarantinedCount: quarantined };
  }, [samples]);

  if (samples === null && !error) {
    return (
      <div className="flex h-40 items-center justify-center text-muted-foreground">
        <Loader2 className="mr-2 h-5 w-5 animate-spin" />
        <span className="text-sm">Loading voice samples…</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-40 flex-col items-center justify-center text-center">
        <p className="text-sm text-muted-foreground">{error}</p>
        <Button variant="outline" size="sm" className="mt-3" onClick={() => void load()}>
          Try again
        </Button>
      </div>
    );
  }

  if (samples && samples.length === 0) {
    return (
      <div className="flex h-40 flex-col items-center justify-center text-center">
        <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-muted">
          <Waves className="h-5 w-5 text-muted-foreground" />
        </div>
        <p className="text-sm font-medium text-foreground">No voice samples yet</p>
        <p className="mt-1 max-w-xs text-xs text-muted-foreground">
          Nixon stores a sample when you confirm {label} on a speaker in a transcript.
        </p>
      </div>
    );
  }

  return (
    <>
      <p className="u-meta">
        {activeCount} active
        {quarantinedCount > 0 ? ` · ${quarantinedCount} quarantined` : ''}
      </p>
      <div className={cn('-mx-1 px-1', bounded && 'max-h-[52vh] overflow-y-auto')}>
        <ul className="space-y-2 py-1">
          {samples?.map((sample) => {
            const isBusy = !!busy[sample.id];
            const date = formatMeetingDate(sample.createdAt);
            const quality = formatQuality(sample.sampleQuality);
            const meetingTitle = sample.sourceMeetingId
              ? meetingTitles[sample.sourceMeetingId]
              : undefined;
            const source = sample.sourceMeetingId
              ? meetingTitle
                ? `From “${meetingTitle}”`
                : 'From a recorded meeting'
              : 'Earlier sample';
            const meta = [source, date, quality].filter(Boolean).join(' · ');
            const isConfirming = confirmingDelete === sample.id;

            return (
              <li
                key={sample.id}
                className={cn(
                  'rounded-lg border px-3 py-2.5 transition-colors',
                  sample.quarantined ? 'border-border/60 bg-muted/40' : 'border-border',
                )}
              >
                <div className="flex items-start gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          'inline-flex items-center gap-1 rounded-[2px] px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide',
                          sample.quarantined
                            ? 'bg-brand/15 text-brand'
                            : 'bg-success/15 text-success',
                        )}
                      >
                        {sample.quarantined ? (
                          <>
                            <ShieldOff className="h-3 w-3" />
                            Quarantined
                          </>
                        ) : (
                          'Active'
                        )}
                      </span>
                    </div>
                    <p className="mt-1 truncate text-xs text-muted-foreground">{meta}</p>
                  </div>

                  {!isConfirming && (
                    <div className="flex flex-shrink-0 items-center gap-1">
                      {sample.quarantined ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1 px-2 text-xs"
                          disabled={isBusy}
                          onClick={() => void toggleQuarantine(sample, false)}
                        >
                          {isBusy ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <RotateCcw className="h-3.5 w-3.5" />
                          )}
                          Restore
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1 px-2 text-xs"
                          disabled={isBusy}
                          onClick={() => void toggleQuarantine(sample, true)}
                        >
                          {isBusy ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <ShieldOff className="h-3.5 w-3.5" />
                          )}
                          Quarantine
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label="Delete this voice sample permanently"
                        title="Delete permanently"
                        className="h-7 w-7 px-0 text-muted-foreground hover:text-destructive"
                        disabled={isBusy}
                        onClick={() => setConfirmingDelete(sample.id)}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  )}
                </div>

                {isConfirming && (
                  <div className="mt-2.5 flex items-center gap-2 rounded-md bg-destructive/10 px-2.5 py-2">
                    <AlertTriangle className="h-4 w-4 flex-shrink-0 text-destructive" />
                    <p className="min-w-0 flex-1 text-xs text-foreground">
                      Delete this sample forever? This can&apos;t be undone.
                    </p>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs"
                      disabled={isBusy}
                      onClick={() => setConfirmingDelete(null)}
                    >
                      Cancel
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      className="h-7 gap-1 px-2 text-xs"
                      disabled={isBusy}
                      onClick={() => void handleDelete(sample)}
                    >
                      {isBusy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                      Delete
                    </Button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </>
  );
}
