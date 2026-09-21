import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// specs/0041 WS3 — the calendar-linked detail-page record control. Locks the three
// states the 1.8 dogfooding bug report is about:
//   1. scheduled + join link  → joinAndRecord receives the URL (the call OPENS);
//   2. scheduled, no link     → record-only (URL null), the documented degrade;
//   3. recorded w/ transcripts → "Continue recording" through the specs/0037 resume
//      path (same meeting id — never a duplicate row via joinAndRecord's create).
// The join-URL resolver + IPC layer are mocked; the control's mode/label/handler
// selection is what's under test.

const {
  joinAndRecordMock,
  openZoomMeetingMock,
  resolveOccurrenceJoinUrlMock,
  armResumeRecordingMock,
  pushMock,
  handleRecordingToggleMock,
} = vi.hoisted(() => ({
  joinAndRecordMock: vi.fn(),
  openZoomMeetingMock: vi.fn(),
  resolveOccurrenceJoinUrlMock: vi.fn(),
  armResumeRecordingMock: vi.fn(),
  pushMock: vi.fn(),
  handleRecordingToggleMock: vi.fn(),
}));

vi.mock('@/lib/calendar', () => ({
  joinAndRecord: joinAndRecordMock,
  openZoomMeeting: openZoomMeetingMock,
  resolveOccurrenceJoinUrl: resolveOccurrenceJoinUrlMock,
  // Real implementation (not a mock): the manual-entry startsAt tests below assert on
  // its actual now-vs-scheduled behavior, not just that it was called. Mirrors
  // lib/calendar.ts's `resolveRecordingStartsAt` exactly — see the sabotage check in
  // that test for proof this isn't a decorative copy.
  resolveRecordingStartsAt: (scheduledStartsAt: string, now: Date) =>
    now.getTime() < new Date(scheduledStartsAt).getTime() ? now.toISOString() : scheduledStartsAt,
  formatRelativeStart: () => 'in 5m',
  // 0 so the open-call → resume settle delay collapses to the next tick in tests.
  JOIN_AND_RECORD_DELAY_MS: 0,
}));
vi.mock('@/lib/resume-recording', () => ({ armResumeRecording: armResumeRecordingMock }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: handleRecordingToggleMock }),
}));

// Not recording by default — the button is disabled while any recording is live.
const recordingStateMock = { isRecording: false };
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => recordingStateMock,
}));

import { ScheduledRecordControl } from '@/components/MeetingDetails/ScheduledRecordControl';

const ZOOM_URL = 'https://zoom.us/j/1234567890?pwd=abc';

/** An occurrence that started a minute ago: inside the record window, not stale. */
function startedJustNow(): string {
  return new Date(Date.now() - 60_000).toISOString();
}

function renderControl(overrides: Partial<Parameters<typeof ScheduledRecordControl>[0]> = {}) {
  return render(
    <ScheduledRecordControl
      meetingId="meeting-1"
      startsAt={startedJustNow()}
      title="Weekly Sync"
      calendarEventId="evt-1"
      seriesKey="series-1"
      origin="scheduled"
      hasTranscripts={false}
      folderPath={null}
      {...overrides}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  recordingStateMock.isRecording = false;
  resolveOccurrenceJoinUrlMock.mockResolvedValue(null);
});

describe('ScheduledRecordControl — start mode (scheduled occurrence)', () => {
  it('resolves the join link and passes it to joinAndRecord (the call opens)', async () => {
    resolveOccurrenceJoinUrlMock.mockResolvedValue(ZOOM_URL);
    const startsAt = startedJustNow();
    renderControl({ startsAt });

    // Link resolved → the label makes the join explicit.
    const button = await screen.findByRole('button', { name: /join & record/i });
    fireEvent.click(button);

    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    const [event, isRecording] = joinAndRecordMock.mock.calls[0];
    expect(event).toEqual({
      id: 'evt-1',
      title: 'Weekly Sync',
      zoomUrl: ZOOM_URL,
      // Timestamp hygiene: re-emitted as strict RFC3339 so api_create_meeting's
      // occurrence-day match never silently falls back to now.
      startsAt: new Date(startsAt).toISOString(),
      seriesKey: 'series-1',
    });
    expect(isRecording).toBe(false);
    // The resolver was pointed at THIS occurrence.
    expect(resolveOccurrenceJoinUrlMock).toHaveBeenCalledWith({
      calendarEventId: 'evt-1',
      occurrenceStart: startsAt,
      meetingId: 'meeting-1',
      seriesKey: 'series-1',
    });
  });

  it('records without a URL when the event has no join link (record-only degrade)', async () => {
    resolveOccurrenceJoinUrlMock.mockResolvedValue(null);
    renderControl();

    const button = await screen.findByRole('button', { name: /start & record/i });
    fireEvent.click(button);

    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    expect(joinAndRecordMock.mock.calls[0][0].zoomUrl).toBeNull();
    expect(armResumeRecordingMock).not.toHaveBeenCalled();
  });

  it('skips join-URL resolution for a stale occurrence (> 1h past start) — no day-agenda fetch', async () => {
    // Opening a months-old calendar-linked meeting must NOT walk the whole day agenda
    // (api_get_day_agenda can trigger a Google network sync) just to resolve a join
    // link there's no call left to join. The button degrades to record-only.
    renderControl({ startsAt: new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString() });

    // Start mode still renders (canRecord is true for all past meetings) …
    const button = await screen.findByRole('button', { name: /start & record/i });
    // … but the resolver was never invoked.
    expect(resolveOccurrenceJoinUrlMock).not.toHaveBeenCalled();

    // And the click records without a URL (the documented degrade).
    fireEvent.click(button);
    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    expect(joinAndRecordMock.mock.calls[0][0].zoomUrl).toBeNull();
  });

  it('treats a recorded row that never captured anything (no folder, no transcripts) as start mode', async () => {
    // Pure false start where recording never began: joinAndRecord's create path is
    // safe — find_adoptable_calendar_meeting reuses the empty row (6h window).
    renderControl({ origin: 'recorded', hasTranscripts: false, folderPath: null });

    const button = await screen.findByRole('button', { name: /start & record/i });
    fireEvent.click(button);
    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    expect(armResumeRecordingMock).not.toHaveBeenCalled();
  });
});

describe('ScheduledRecordControl — manual entries bypass the T-5 window (specs/0069b followup)', () => {
  it('shows the Record button for a manual entry days out, unlike a calendar-backed one', async () => {
    const farFuture = new Date(Date.now() + 3 * 24 * 60 * 60 * 1000).toISOString(); // 3 days out

    // Calendar-backed (default isManualEntry: false): 3 days out is well outside the
    // T-5 window, so no button renders.
    const { unmount } = renderControl({ startsAt: farFuture });
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    unmount();

    // Same far-future start, but a manual entry: the button is there anyway.
    renderControl({ startsAt: farFuture, isManualEntry: true });
    expect(await screen.findByRole('button', { name: /start & record/i })).toBeInTheDocument();
  });

  it('sends now (not the far-future scheduled start) when recording a manual entry early', async () => {
    // Far enough out that the test cannot pass by proximity to "now".
    const farFuture = new Date(Date.now() + 3 * 24 * 60 * 60 * 1000).toISOString();
    renderControl({ startsAt: farFuture, isManualEntry: true });

    const button = await screen.findByRole('button', { name: /start & record/i });
    const beforeClick = Date.now();
    fireEvent.click(button);

    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    const sentStartsAt = joinAndRecordMock.mock.calls[0][0].startsAt as string;
    expect(sentStartsAt).not.toBe(farFuture);
    expect(Math.abs(new Date(sentStartsAt).getTime() - beforeClick)).toBeLessThan(5000);
  });

  it('sends the scheduled start for a manual entry once it has already passed', async () => {
    const past = new Date(Date.now() - 60_000).toISOString(); // started a minute ago
    renderControl({ startsAt: past, isManualEntry: true });

    const button = await screen.findByRole('button', { name: /start & record/i });
    fireEvent.click(button);

    expect(joinAndRecordMock).toHaveBeenCalledTimes(1);
    expect(joinAndRecordMock.mock.calls[0][0].startsAt).toBe(new Date(past).toISOString());
  });
});

describe('ScheduledRecordControl — continue mode (occurrence already recorded)', () => {
  it('offers "Continue recording" and resumes into the SAME meeting (specs/0037), no create', async () => {
    renderControl({
      origin: 'recorded',
      hasTranscripts: true,
      folderPath: '/tmp/rec/meeting-1',
    });

    const button = await screen.findByRole('button', { name: /continue recording/i });
    fireEvent.click(button);

    // No join link → resume immediately: same meeting id + folder, then /record.
    expect(armResumeRecordingMock).toHaveBeenCalledWith({
      meetingId: 'meeting-1',
      folderPath: '/tmp/rec/meeting-1',
      meetingName: 'Weekly Sync',
    });
    expect(pushMock).toHaveBeenCalledWith('/record');
    // Never through the create path — that's what minted the duplicate row.
    expect(joinAndRecordMock).not.toHaveBeenCalled();
  });

  it('re-opens the call first when the event has a link, then arms the resume', async () => {
    resolveOccurrenceJoinUrlMock.mockResolvedValue(ZOOM_URL);
    renderControl({
      origin: 'recorded',
      hasTranscripts: true,
      folderPath: '/tmp/rec/meeting-1',
    });

    const button = await screen.findByRole('button', { name: /continue recording/i });
    // Wait for the link to resolve so the click takes the open-call branch.
    await waitFor(() => expect(resolveOccurrenceJoinUrlMock).toHaveBeenCalled());
    fireEvent.click(button);

    expect(openZoomMeetingMock).toHaveBeenCalledWith(ZOOM_URL);
    // The resume is armed after the (test-collapsed) settle delay.
    await waitFor(() =>
      expect(armResumeRecordingMock).toHaveBeenCalledWith({
        meetingId: 'meeting-1',
        folderPath: '/tmp/rec/meeting-1',
        meetingName: 'Weekly Sync',
      }),
    );
    expect(pushMock).toHaveBeenCalledWith('/record');
    expect(joinAndRecordMock).not.toHaveBeenCalled();
  });

  it('renders nothing once the occurrence is stale (> 1h past start) — the "…" menu owns late appends', () => {
    const { container } = renderControl({
      origin: 'recorded',
      hasTranscripts: true,
      folderPath: '/tmp/rec/meeting-1',
      startsAt: new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString(),
    });
    expect(container).toBeEmptyDOMElement();
  });

  it('disables the button while another recording is live', async () => {
    recordingStateMock.isRecording = true;
    renderControl({
      origin: 'recorded',
      hasTranscripts: true,
      folderPath: '/tmp/rec/meeting-1',
    });

    const button = await screen.findByRole('button', { name: /recording…/i });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(armResumeRecordingMock).not.toHaveBeenCalled();
    expect(joinAndRecordMock).not.toHaveBeenCalled();
  });
});
