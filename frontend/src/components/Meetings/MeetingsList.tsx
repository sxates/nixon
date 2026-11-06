'use client';

import { useCallback, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Loader2, Mic } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import { avatarColorClass } from '@/lib/avatar-colors';
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
 * in day cards matching the Week view (specs/0054 W3 follow-up) — the bare label + rows
 * read as one undifferentiated stream once there are hundreds of meetings.
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
    return (
      <div className="flex h-64 flex-shrink-0 items-center justify-center text-muted-foreground">
        <Loader2 className="mr-2 h-5 w-5 animate-spin" />
        <span className="text-sm">Loading meetings…</span>
      </div>
    );
  }
  if (error) {
    return (
      <div className="flex h-64 flex-col items-center justify-center text-center">
        <p className="text-sm text-muted-foreground">{error}</p>
      </div>
    );
  }
  if (meetings.length === 0) {
    return (
      <div className="flex h-[40vh] flex-col items-center justify-center text-center">
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-muted">
          <Mic className="h-6 w-6 text-muted-foreground" />
        </div>
        <h2 className="text-base font-semibold text-foreground">No recordings yet</h2>
        <p className="mt-1 max-w-xs text-sm text-muted-foreground">
          Start recording and your meetings will show up here.
        </p>
        <Button variant="brand" onClick={() => router.push('/record')} className="mt-5 gap-2">
          <Mic className="h-4 w-4" />
          Open the recorder
        </Button>
      </div>
    );
  }

  return (
    <>
      <div className="space-y-2.5 pb-2">
        {dayGroups.map((group) => (
          <div
            key={group.key}
            className={`overflow-hidden rounded-[3px] border bg-card ${
              group.isToday ? 'border-brand/45' : 'border-border'
            }`}
          >
            <div className="flex items-center justify-between gap-3 px-4 py-2">
              <span className="flex min-w-0 items-baseline gap-2">
                <span className={`u-section-label ${group.isToday ? 'text-brand' : 'text-foreground'}`}>
                  {group.label}
                </span>
                {group.dateLabel && (
                  <span className="u-section-label text-muted-foreground">{group.dateLabel}</span>
                )}
              </span>
              <span className="u-section-label flex-shrink-0">
                {group.items.length} recording{group.items.length === 1 ? '' : 's'}
              </span>
            </div>
            <div className="border-t border-border/50 [&>*:last-child]:border-b-0">
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
            </div>
          </div>
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
