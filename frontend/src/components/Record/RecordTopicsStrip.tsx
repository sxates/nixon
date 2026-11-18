'use client';

import { MeetingTopicsRow } from '@/components/Topics/MeetingTopicsRow';
import { RECORD_STRIP_HEIGHT } from '@/lib/record-view';
import { cn } from '@/lib/utils';

/**
 * The live Meeting's Topics row at the top of the record screen's right-hand panel (owner
 * feedback, v0.18): the Notes / Agenda / To-Dos rail in Transcript, the To-Do sidebar in
 * My Notes / Agenda. Linking a Topic mid-recording needs nothing more: the Topic entry is written after
 * the summary completes, when the summary job reads the Meeting's links. Hidden until the
 * meeting row exists.
 */
export function RecordTopicsStrip({
  meetingId,
  meetingTitle,
}: {
  meetingId: string | null;
  meetingTitle: string | null;
}) {
  if (!meetingId) return null;
  return (
    // RECORD_STRIP_HEIGHT: the same 44px band as the row beside it (the main column's top
    // strip in My Notes / Agenda, the transcript's status strip in Transcript), so the
    // rules line up.
    <div className={cn('flex flex-shrink-0 items-center border-b border-border px-5 py-1.5', RECORD_STRIP_HEIGHT)}>
      <MeetingTopicsRow meetingId={meetingId} title={meetingTitle} seriesKey={null} className="min-w-0 flex-1" />
    </div>
  );
}
