/**
 * specs/0038 WS6.c — frontend refresh signal for the meeting participant roster.
 *
 * Assigning a transcript speaker to a person (via `useSpeakers`) now also adds that person to
 * the meeting's `meeting_participants` roster on the backend. The speaker surfaces
 * (SpeakerLegend / InlineSpeakerAssign, both routed through `useSpeakers`) and the
 * `ParticipantsPanel` are decoupled siblings in the meeting-details tree, so rather than
 * threading a refetch callback through the whole tree we bridge them with a window event —
 * the same pattern the recording start/stop flows already use
 * (`start-recording-from-sidebar`, `stop-recording-from-global-bar`).
 *
 * The panel listens for this event and re-fetches its roster when the meeting id matches, so a
 * newly-identified person appears without a manual reload.
 */
export const MEETING_PARTICIPANTS_CHANGED_EVENT = 'meeting-participants-changed';

/** Fire after a mutation that may have changed a meeting's participant roster. */
export function notifyMeetingParticipantsChanged(meetingId: string): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent(MEETING_PARTICIPANTS_CHANGED_EVENT, { detail: { meetingId } }),
  );
}
