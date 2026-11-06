'use client';

import { useRouter } from 'next/navigation';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import { AddMeetingDialog } from '@/components/Today/AddMeetingDialog';
import { DeleteManualMeetingDialog } from '@/components/Today/DeleteManualMeetingDialog';
import { openMeetingUrl } from '@/lib/today-timeline';
import type { AgendaActions } from '@/hooks/useAgendaActions';

/**
 * The Meetings page's add / edit / delete dialogs for agenda rows, driven by
 * `useAgendaActions`. `onChanged` re-reads whatever view is showing.
 */
export function AgendaDialogs({
  actions,
  defaultDateKey,
  onChanged,
}: {
  actions: AgendaActions;
  defaultDateKey: string;
  onChanged: () => void;
}) {
  const router = useRouter();
  const {
    addOpen,
    editingItem,
    closeManualDialog,
    deletingItem,
    setDeletingItem,
    deletingRecorded,
    setDeletingRecorded,
  } = actions;

  return (
    <>
      <AddMeetingDialog
        open={addOpen || !!editingItem}
        onOpenChange={closeManualDialog}
        defaultDateKey={defaultDateKey}
        editing={
          editingItem
            ? {
                meetingId: editingItem.meetingId ?? editingItem.id,
                title: editingItem.title,
                startsAt: editingItem.startTime,
                endsAt: editingItem.endTime,
                joinUrl: editingItem.zoomUrl,
              }
            : undefined
        }
        // specs/0069b followup — a fresh create carries its new id (see AddMeetingDialog's
        // `onSaved`) and jumps straight to that meeting's Prep tab, the same place clicking
        // it on the timeline would land (`routeForItem`'s manual-entry branch). Editing never
        // carries an id, so it stays here. The create path skips the refresh: we're
        // navigating away and unmounting this tree, so re-reading the now-stale agenda would
        // be wasted work racing the navigation (and risks setting state after unmount).
        onSaved={(createdMeetingId) => {
          if (createdMeetingId) {
            router.push(openMeetingUrl(createdMeetingId, 'prep'));
            return;
          }
          onChanged();
        }}
      />

      <DeleteManualMeetingDialog
        open={!!deletingItem}
        onOpenChange={(open) => {
          if (!open) setDeletingItem(null);
        }}
        meetingId={deletingItem?.meetingId ?? deletingItem?.id ?? null}
        meetingTitle={deletingItem?.title}
        onDeleted={() => {
          setDeletingItem(null);
          onChanged();
        }}
      />

      {deletingRecorded?.meetingId && (
        <DeleteMeetingDialog
          open
          onOpenChange={(open) => {
            if (!open) setDeletingRecorded(null);
          }}
          meetingId={deletingRecorded.meetingId}
          meetingTitle={deletingRecorded.title}
          onDeleted={() => {
            setDeletingRecorded(null);
            onChanged();
          }}
        />
      )}
    </>
  );
}
