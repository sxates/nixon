'use client';

import { useCallback, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import { avatarColorClass } from '@/lib/avatar-colors';
import { DayGroupCard } from './DayGroupCard';
import { dayHeading, dayKey, MeetingRow, type DashboardMeeting } from './MeetingRow';

interface DayGroup {
  key: string;
  label: string;
  dateLabel: string;
  isToday: boolean;
  items: DashboardMeeting[];
}

/**
 * Group meetings by calendar day (specs/0038 feedback #5): days newest first, and within a
 * day the meetings in the order they happened — the order the Agenda and Week views use.
 * Sorted here rather than trusted from the caller: this used to rely on the input already
 * being newest-first, and drew days out of place whenever it wasn't (the screenshot mock's
 * wasn't). Undated rows (unparseable timestamp) collect into a trailing "Undated" group.
 */
export function groupByDay(meetings: DashboardMeeting[], now: Date = new Date()): DayGroup[] {
  const order: string[] = [];
  const byKey = new Map<string, Omit<DayGroup, 'key'>>();
  const undated: DashboardMeeting[] = [];
  const time = (m: DashboardMeeting) => new Date(m.createdAt).getTime();
  const sorted = [...meetings].sort((a, b) => {
    const ta = time(a);
    const tb = time(b);
    if (Number.isNaN(ta) || Number.isNaN(tb)) return 0;
    const da = dayKey(new Date(ta));
    const db = dayKey(new Date(tb));
    if (da !== db) return da < db ? 1 : -1; // newer day first
    return ta - tb; // earlier in the day first
  });

  for (const m of sorted) {
    const created = new Date(m.createdAt);
    if (Number.isNaN(created.getTime())) {
      undated.push(m);
      continue;
    }
    const key = dayKey(created);
    let group = byKey.get(key);
    if (!group) {
      const heading = dayHeading(created, now);
      group = {
        label: heading.primary,
        dateLabel: heading.secondary,
        isToday: key === dayKey(now),
        items: [],
      };
      byKey.set(key, group);
      order.push(key);
    }
    group.items.push(m);
  }

  const groups: DayGroup[] = order.map((key) => ({ key, ...byKey.get(key)! }));
  if (undated.length > 0) {
    groups.push({ key: 'undated', label: 'Undated', dateLabel: '', isToday: false, items: undated });
  }
  return groups;
}

/**
 * The Meetings List view (the former All meetings page): every recording, newest first,
 * in day cards — the Week view's own `DayGroupCard` (specs/0054 W3 follow-up) — the bare
 * label + rows read as one undifferentiated stream once there are hundreds of meetings.
 */
export function MeetingsList({
  meetings,
  isLoading,
  error,
  onDeleted,
}: {
  meetings: DashboardMeeting[];
  isLoading: boolean;
  error: string | null;
  onDeleted: (id: string) => void;
}) {
  const router = useRouter();
  const { refetchMeetings, activeRecordingMeetingId } = useSidebar();
  const { isRecording } = useRecordingState();
  const processingIds = useProcessingMeetingIds();
  // Meeting pending deletion (drives the confirm dialog); null when closed.
  const [meetingToDelete, setMeetingToDelete] = useState<DashboardMeeting | null>(null);

  const dayGroups = useMemo(() => groupByDay(meetings), [meetings]);

  const handleOpen = useCallback(
    (id: string) => router.push(`/meeting-details?id=${id}`),
    [router],
  );

  const handleDeleted = useCallback(async () => {
    if (meetingToDelete) onDeleted(meetingToDelete.id);
    await refetchMeetings();
  }, [meetingToDelete, onDeleted, refetchMeetings]);

  if (isLoading) {
    return <PageLoading fill="pane" label="Loading meetings…" />;
  }
  if (error) {
    return (
      <Callout tone="error" className="mt-2">
        {error}
      </Callout>
    );
  }
  if (meetings.length === 0) {
    return (
      <EmptyState
        className="h-[40vh]"
        title="No recordings yet"
        body="Start recording and your meetings will show up here."
        action={
          <Button variant="brand" onClick={() => router.push('/record')}>
            Open the recorder
          </Button>
        }
      />
    );
  }

  return (
    <>
      <div className="flex flex-col gap-3 pb-2">
        {dayGroups.map((group) => (
          <DayGroupCard
            key={group.key}
            label={group.label}
            dateLabel={group.dateLabel}
            count={group.items.length}
            isToday={group.isToday}
          >
            {group.items.map((m) => (
              <MeetingRow
                key={m.id}
                meeting={m}
                spineClass={avatarColorClass(m.id)}
                isRecordingThis={isRecording && m.id === activeRecordingMeetingId}
                isProcessing={processingIds.has(m.id)}
                onOpen={handleOpen}
                onRequestDelete={setMeetingToDelete}
              />
            ))}
          </DayGroupCard>
        ))}
      </div>

      {meetingToDelete && (
        <DeleteMeetingDialog
          open
          onOpenChange={(open) => {
            if (!open) setMeetingToDelete(null);
          }}
          meetingId={meetingToDelete.id}
          meetingTitle={meetingToDelete.title}
          onDeleted={handleDeleted}
        />
      )}
    </>
  );
}
