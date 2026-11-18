'use client';

import { MeetingTopicsRow } from '@/components/Topics/MeetingTopicsRow';

/**
 * The live Meeting's Topics row at the top of the record screen's right-hand panel (owner
 * feedback, v0.18): the Notes / Agenda / To-Dos rail in Transcript, the To-Do sidebar in
 * Focus. Linking a Topic mid-recording needs nothing more: the Topic entry is written after
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
    <div className="flex-shrink-0 border-b border-border px-5 py-2">
      <MeetingTopicsRow meetingId={meetingId} title={meetingTitle} seriesKey={null} className="" />
    </div>
  );
}
