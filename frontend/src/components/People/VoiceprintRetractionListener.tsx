'use client';

/**
 * Voiceprint retraction feedback (specs/0039 WS3, task 9).
 *
 * App-wide background component (mounted in layout.tsx alongside ZoomAutoDetect /
 * CalendarAlerts) that listens for the backend `voiceprint-retracted` event. When a WS2 span
 * correction reassigns a transcript span away from a speaker, the backend quarantines the voice
 * samples that speaker's cluster contributed for that meeting and emits this event — once per
 * affected person. We surface a sonner toast ("Removed a voiceprint sample from {name}") with an
 * **Undo** action that restores exactly those quarantined samples, so an over-eager retraction is
 * a one-click fix. Quarantine (not delete) is what the backend does, so Undo is always safe.
 *
 * Aggregation: a single correction can emit several events for the SAME person (e.g. multiple
 * spans). We key the toast by `personId` (stable toast id) and accumulate that person's
 * quarantined ids, so repeated events update ONE toast in place instead of stacking N identical
 * ones — and Undo restores every accumulated id.
 *
 * Global by design so the feedback shows regardless of the current route (the user might have
 * navigated away from the meeting after correcting it). Listener uses `safeListen` so teardown is
 * race-safe / crash-proof.
 */

import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';

interface VoiceprintRetractedEvent {
  meetingId: string;
  personId: string;
  personName: string;
  quarantinedSampleIds: string[];
}

const toastId = (personId: string) => `voiceprint-retracted-${personId}`;

export default function VoiceprintRetractionListener() {
  // personId → the sample ids currently represented by that person's active toast, so
  // repeated events merge and Undo restores all of them. Kept in a ref because the
  // long-lived listener is set up once on mount.
  const pendingRef = useRef<Map<string, { name: string; ids: Set<string> }>>(new Map());

  useEffect(() => {
    // Render (or update-in-place) the undo toast for a person from their current pending
    // set. Reused both when a retraction event arrives and when a partial-failure Undo
    // needs to re-offer itself for the samples that couldn't be restored.
    const showRetractionToast = (personId: string) => {
      const entry = pendingRef.current.get(personId);
      if (!entry || entry.ids.size === 0) return;
      const { name, ids } = entry;
      const count = ids.size;
      const id = toastId(personId);

      toast(
        count === 1
          ? `Removed a voice sample from ${name}`
          : `Removed ${count} voice samples from ${name}`,
        {
          id,
          description: 'A correction suggested this voice no longer matches.',
          action: {
            label: 'Undo',
            onClick: () => void restoreAll(personId),
          },
          onAutoClose: () => pendingRef.current.delete(personId),
          onDismiss: () => pendingRef.current.delete(personId),
        },
      );
    };

    // Undo: restore every sample currently pending for this person. Uses allSettled so one
    // failing restore doesn't sink the successes — only the ids that actually restored are
    // dropped from `pendingRef`; the rest stay pending and the toast re-shows so Undo stays
    // available (retryable) for them. State is cleared only AFTER the calls settle.
    const restoreAll = async (personId: string) => {
      const entry = pendingRef.current.get(personId);
      const name = entry?.name ?? 'this person';
      const toRestore = Array.from(entry?.ids ?? []);
      if (toRestore.length === 0) return;

      const results = await Promise.allSettled(
        toRestore.map((sampleId) =>
          invoke('api_restore_voiceprint_sample', { sampleId }),
        ),
      );

      const failed: string[] = [];
      let restoredCount = 0;
      results.forEach((result, i) => {
        if (result.status === 'fulfilled') restoredCount += 1;
        else failed.push(toRestore[i]);
      });

      if (failed.length === 0) {
        // Everything restored — clear the pending set and confirm.
        pendingRef.current.delete(personId);
        toast.dismiss(toastId(personId));
        toast.success(
          restoredCount === 1
            ? `Restored a voice sample for ${name}`
            : `Restored ${restoredCount} voice samples for ${name}`,
        );
        return;
      }

      // Partial (or total) failure: keep ONLY the still-quarantined ids pending, then
      // re-show the toast so Undo can be retried for exactly those.
      pendingRef.current.set(personId, { name, ids: new Set(failed) });
      const reason = results.find(
        (r): r is PromiseRejectedResult => r.status === 'rejected',
      )?.reason;
      console.error('Failed to restore some retracted voice sample(s):', reason);
      toast.error(
        restoredCount > 0
          ? `Restored ${restoredCount}, but ${failed.length} couldn't be undone — retry Undo, or use the People page.`
          : "Couldn't undo — retry, or restore from the People page.",
        { description: reason instanceof Error ? reason.message : String(reason) },
      );
      showRetractionToast(personId);
    };

    return safeListen<VoiceprintRetractedEvent>('voiceprint-retracted', (event) => {
      const payload = event.payload;
      const newIds = (payload?.quarantinedSampleIds ?? []).filter(Boolean);
      if (!payload?.personId || newIds.length === 0) return;

      const { personId } = payload;
      const name = payload.personName?.trim() || 'this person';

      // Merge into any still-showing toast for this person.
      const existing = pendingRef.current.get(personId);
      const ids = existing?.ids ?? new Set<string>();
      newIds.forEach((id) => ids.add(id));
      pendingRef.current.set(personId, { name, ids });

      showRetractionToast(personId);
    });
  }, []);

  return null;
}
