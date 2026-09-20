import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor } from '@testing-library/react';

// specs/0068 W3 — two alerts per meeting: one at T-5, one as it starts. The decisions worth
// pinning are which one fires when, that neither fires twice, and that the start alert
// stands down while Nixon is already recording.

const { notifyMock, getStatusMock, getUpcomingMock, joinAndRecordMock, pushMock, prepRouteMock } =
  vi.hoisted(() => ({
    notifyMock: vi.fn().mockResolvedValue(true),
    getStatusMock: vi.fn().mockResolvedValue('authorized'),
    getUpcomingMock: vi.fn(),
    joinAndRecordMock: vi.fn(),
    pushMock: vi.fn(),
    prepRouteMock: vi.fn().mockResolvedValue('/meeting-details?id=minted&tab=prep'),
  }));

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));
vi.mock('@/lib/prep', () => ({ prepRouteForEvent: prepRouteMock }));

vi.mock('@/lib/osNotification', async () => {
  const actual = await vi.importActual<typeof import('@/lib/osNotification')>('@/lib/osNotification');
  return { ...actual, notify: notifyMock, focusMainWindow: vi.fn() };
});
vi.mock('@/lib/calendar', () => ({
  getCalendarAccessStatus: getStatusMock,
  getUpcomingMeetings: getUpcomingMock,
  joinAndRecord: joinAndRecordMock,
  openZoomMeeting: vi.fn(),
  formatClockTime: () => '10:00',
}));

let recording = false;
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: recording }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: vi.fn() }),
}));

import CalendarAlerts from '@/components/Calendar/CalendarAlerts';

function meetingIn(ms: number) {
  return [
    {
      id: 'm-1',
      title: 'Standup',
      startsAt: new Date(Date.now() + ms).toISOString(),
      endsAt: new Date(Date.now() + ms + 1800000).toISOString(),
      calendarName: 'Work',
      location: null,
      zoomUrl: 'https://zoom.us/j/1',
      externalId: null,
    },
  ];
}

beforeEach(() => {
  vi.clearAllMocks();
  recording = false;
  sessionStorage.clear();
  notifyMock.mockResolvedValue(true);
  getStatusMock.mockResolvedValue('authorized');
});

afterEach(() => {
  vi.useRealTimers();
});

describe('which alert fires', () => {
  it('says "in 5 min" four minutes out', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].title).toMatch(/in \d+ min/);
  });

  it('says "starting now" inside the last minute', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(30 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].title).toBe('Standup starting now');
  });

  // A meeting first seen at T-30s should not be announced as "in 0 min" or "in 1 min":
  // the wording follows the same decision that chose the alert.
  it('never says "in 0 min"', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(20 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].title).not.toMatch(/in 0 min/);
  });

  it('ignores a meeting still far off', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(30 * 60 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(notifyMock).not.toHaveBeenCalled();
  });

  it('ignores a meeting that started long ago', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(-5 * 60 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(notifyMock).not.toHaveBeenCalled();
  });
});

describe('not twice', () => {
  it('does not repeat the same alert on the next poll', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    const { rerender } = render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalledTimes(1));
    rerender(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(notifyMock).toHaveBeenCalledTimes(1);
  });

  // The lead and start alerts share one notification id, so macOS replaces the banner
  // rather than leaving two for the same meeting.
  it('reuses the notification id across both alerts', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].id).toBe('meeting-m-1');
  });
});

describe('while already recording', () => {
  it('stands down rather than announcing the meeting you are in', async () => {
    recording = true;
    getUpcomingMock.mockResolvedValue(meetingIn(20 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(notifyMock).not.toHaveBeenCalled();
  });

  // The five-minute warning is still useful mid-recording: it is about the NEXT meeting.
  it('still gives the five-minute warning', async () => {
    recording = true;
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
  });
});

// A banner can only show one button, so each alert carries the thing worth doing at that
// moment: Prep five minutes out, Join & Record as it starts.
describe('which button each alert carries', () => {
  it('offers Prep on the five-minute warning', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].category).toBe('nixon.prep');
  });

  it('offers Prep even when the meeting has no join link — prep needs no link', async () => {
    getUpcomingMock.mockResolvedValue([{ ...meetingIn(4 * 60 * 1000)[0], zoomUrl: null }]);
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].category).toBe('nixon.prep');
  });

  it('offers Join & Record as the meeting starts', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(20 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].category).toBe('nixon.meeting');
  });

  it('offers plain Record at the start of a meeting with nothing to join', async () => {
    getUpcomingMock.mockResolvedValue([{ ...meetingIn(20 * 1000)[0], zoomUrl: null }]);
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].category).toBe('nixon.record');
  });
});

describe('pressing Prep', () => {
  it('mints the scheduled meeting and routes to its Prep tab', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());

    await notifyMock.mock.calls[0][0].onPrep();
    expect(prepRouteMock).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'm-1', title: 'Standup' }),
    );
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=minted&tab=prep');
  });

  // The body tap should land somewhere useful, not just raise the window.
  it('is also what tapping the warning banner does', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].onOpen).toBe(notifyMock.mock.calls[0][0].onPrep);
  });
});

describe('when the calendar is not available', () => {
  it('does nothing at all', async () => {
    getStatusMock.mockResolvedValue('notDetermined');
    getUpcomingMock.mockResolvedValue(meetingIn(60 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(getUpcomingMock).not.toHaveBeenCalled();
    expect(notifyMock).not.toHaveBeenCalled();
  });
});
