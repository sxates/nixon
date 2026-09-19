import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import type { PrepView } from '@/lib/prep';

/**
 * Whether a meeting has prep worth opening, and how many carried-over items it
 * has (specs/0054 W4).
 *
 * Drives the record screen's Prep button: the button is hidden entirely when
 * there is nothing to show, so an ad-hoc recording's header is unchanged. Reads
 * the same `api_get_prep` view the Prep tab uses and refreshes on the
 * `prep-briefs-updated` broadcast, so a brief that finishes generating mid-call
 * makes the button appear.
 *
 * Failures are non-fatal and silent: prep is an enhancement, and a recording in
 * progress must never surface an error for it.
 */
export function usePrepAvailability(meetingId: string | null | undefined): {
  hasPrep: boolean;
  openItemCount: number;
  /**
   * Re-read the view now. `api_set_action_item_status` emits no event, so checking a
   * carried-over item off in the Prep panel is otherwise invisible to this hook and the
   * count badge would keep showing the old number (specs/0063 W4).
   */
  refresh: () => void;
} {
  const [hasPrep, setHasPrep] = useState(false);
  const [openItemCount, setOpenItemCount] = useState(0);

  const load = useCallback(async () => {
    if (!meetingId) {
      setHasPrep(false);
      setOpenItemCount(0);
      return;
    }
    try {
      const view = await invoke<PrepView | null>('api_get_prep', { meetingId });
      if (!view) {
        setHasPrep(false);
        setOpenItemCount(0);
        return;
      }
      const openItems = view.openItems?.length ?? 0;
      // Deliberately NOT `isBriefLoading` — that also treats 'absent' as loading
      // (it drives the Prep tab's spinner), and 'absent' is the state of every
      // ad-hoc recording, which would put the button on every single meeting.
      // Only real content, or a brief explicitly in flight, counts.
      const hasBrief = !!view.briefMarkdown?.trim() || view.briefStatus === 'pending';
      const hasNotes = !!view.prepNotesMarkdown?.trim();
      setOpenItemCount(openItems);
      setHasPrep(hasBrief || hasNotes || openItems > 0);
    } catch (error) {
      console.warn('Could not check prep availability for the recording:', error);
    }
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => safeListen('prep-briefs-updated', () => void load()), [load]);

  const refresh = useCallback(() => void load(), [load]);

  return { hasPrep, openItemCount, refresh };
}
