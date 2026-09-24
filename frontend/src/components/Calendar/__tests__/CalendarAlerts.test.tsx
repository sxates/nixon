import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor } from '@testing-library/react';

// specs/0068 W3 — two alerts per meeting: one at T-5, one as it starts. The decisions worth
// pinning are which one fires when, that neither fires twice, and that the start alert
// stands down while Nixon is already recording.

const {
  notifyMock,
  cancelPendingMock,
  invokeMock,
  getUpcomingMock,
  joinAndRecordMock,
  pushMock,
  prepRouteMock,
  listeners,
} = vi.hoisted(() => ({
  notifyMock: vi.fn(),
  cancelPendingMock: vi.fn(),
  invokeMock: vi.fn(),
  getUpcomingMock: vi.fn(),
  joinAndRecordMock: vi.fn(),
  pushMock: vi.fn(),
  prepRouteMock: vi.fn().mockResolvedValue('/meeting-details?id=minted&tab=prep'),
  listeners: new Map<string, () => void>(),
}));

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));
vi.mock('@/lib/prep', () => ({ prepRouteForEvent: prepRouteMock }));

vi.mock('@/lib/osNotification', async () => {
  const actual = await vi.importActual<typeof import('@/lib/osNotification')>('@/lib/osNotification');
  return {
    ...actual,
    notify: notifyMock,
    cancelPending: cancelPendingMock,
    focusMainWindow: vi.fn(),
  };
});
// Capture the `google-calendar-synced` handler: the tests use it to run a tick at a chosen
// moment, which is also what a finished sync does.
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, handler: () => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  },
}));
// The connection gate is the REAL `isAnyCalendarConnected` (specs/0074 W5), answered at the
// IPC boundary, so the test pins which sources count rather than a mock of the answer.
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
let eventkitStatus = 'authorized';
let googleConnected = false;
function calendarBackend() {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'api_get_calendar_access_status') return eventkitStatus;
    if (command === 'api_google_calendar_status') {
      return { configured: true, connected: googleConnected, email: null, calendars: [] };
    }
    return null;
  });
}
vi.mock('@/lib/calendar', async () => {
  const actual = await vi.importActual<typeof import('@/lib/calendar')>('@/lib/calendar');
  return {
    isAnyCalendarConnected: actual.isAnyCalendarConnected,
    getUpcomingMeetings: getUpcomingMock,
    tryGetUpcomingMeetings: getUpcomingMock,
    joinAndRecord: joinAndRecordMock,
    openZoomMeeting: vi.fn(),
    formatClockTime: () => '10:00',
  };
});

let recording = false;
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: recording }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: vi.fn() }),
}));

import CalendarAlerts from '@/components/Calendar/CalendarAlerts';

/** Let the pending promise chains (IPC mocks, notify) settle. */
const settle = () => new Promise((r) => setTimeout(r, 20));

/** Run one tick now, the way a finished Google sync does, and let it settle. */
async function tickNow() {
  const handler = listeners.get('google-calendar-synced');
  expect(handler).toBeDefined();
  handler?.();
  await settle();
}

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
  cancelPendingMock.mockResolvedValue(undefined);
  listeners.clear();
  eventkitStatus = 'authorized';
  googleConnected = false;
  calendarBackend();
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

  // The backend returns a just-started meeting only because we ask it to (specs/0075 W2).
  it('asks the backend for meetings that started in the last two minutes', async () => {
    getUpcomingMock.mockResolvedValue([]);
    render(<CalendarAlerts />);
    await waitFor(() => expect(getUpcomingMock).toHaveBeenCalled());
    expect(getUpcomingMock).toHaveBeenCalledWith(12, { includeStartedWithinMs: 120_000 });
  });

  // The in-app backstop: first seen 90s after the start (no warning ever fired).
  it('still says "starting now" just after the start', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(-90 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].title).toBe('Standup starting now');
  });

  it('ignores a meeting that started long ago', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(-5 * 60 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(notifyMock).not.toHaveBeenCalled();
  });
});

describe('not twice', () => {
  // Two notify calls at T-5: the warning, and the start banner handed to macOS for later.
  it('does not repeat the same alert on the next poll', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalledTimes(2));
    await tickNow();
    expect(notifyMock).toHaveBeenCalledTimes(2);
  });

  // specs/0075 W2: the scheduled start banner has its own id, so delivering the T-5 banner
  // (`meeting-<id>`) cannot replace the pending start request.
  it('gives the warning and the start banner different ids', async () => {
    getUpcomingMock.mockResolvedValue(meetingIn(4 * 60 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalledTimes(2));
    expect(notifyMock.mock.calls[0][0].id).toBe('meeting-m-1');
    expect(notifyMock.mock.calls[1][0].id).toBe('meeting-start-m-1');
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

describe('which calendars count', () => {
  // Google-only users got no alerts at all while the gate read EventKit alone.
  it('alerts with only Google connected', async () => {
    eventkitStatus = 'notDetermined';
    googleConnected = true;
    getUpcomingMock.mockResolvedValue(meetingIn(30 * 1000));
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    expect(notifyMock.mock.calls[0][0].title).toBe('Standup starting now');
  });
});

describe('when the calendar is not available', () => {
  it('does nothing at all', async () => {
    eventkitStatus = 'notDetermined';
    getUpcomingMock.mockResolvedValue(meetingIn(60 * 1000));
    render(<CalendarAlerts />);
    await new Promise((r) => setTimeout(r, 20));
    expect(getUpcomingMock).not.toHaveBeenCalled();
    expect(notifyMock).not.toHaveBeenCalled();
  });
});

// specs/0075 W2 — the start banner is scheduled with macOS when the warning fires, because
// a backgrounded webview's ticks arrive 61–181s apart and used to skip the 60s start window.
describe('the scheduled start banner', () => {
  const T0 = new Date('2026-09-23T15:00:00Z').getTime();
  const startsAt = new Date(T0).toISOString();

  function meetingAt(start: string) {
    return [{ ...meetingIn(0)[0], startsAt: start, endsAt: new Date(T0 + 1800000).toISOString() }];
  }
  const startBanners = () =>
    notifyMock.mock.calls.map((c) => c[0]).filter((o) => o.id === 'meeting-start-m-1');

  beforeEach(() => {
    // Only the clock: RTL's waitFor needs real timers.
    vi.useFakeTimers({ toFake: ['Date'] });
    getUpcomingMock.mockResolvedValue(meetingAt(startsAt));
  });

  async function warnAt(msBefore: number) {
    vi.setSystemTime(T0 - msBefore);
    const view = render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalledTimes(2));
    return view;
  }

  it('is handed to macOS for the start time when the warning fires', async () => {
    await warnAt(4 * 60 * 1000);
    const [start] = startBanners();
    expect(start).toMatchObject({
      title: 'Standup starting now',
      category: 'nixon.meeting',
      deliverAtMs: T0,
    });
    // The warning itself is delivered now.
    expect(notifyMock.mock.calls[0][0].deliverAtMs).toBeUndefined();
  });

  // The owner's report: one tick at T-61s, the next at T+70s.
  it('survives a late tick: exactly one start banner, no second one in-app', async () => {
    await warnAt(61 * 1000);
    expect(startBanners()).toHaveLength(1);
    expect(startBanners()[0].deliverAtMs).toBe(T0);

    vi.setSystemTime(T0 + 70 * 1000);
    await tickNow();
    expect(startBanners()).toHaveLength(1);
    expect(notifyMock).toHaveBeenCalledTimes(2);
  });

  // Choice (3): the in-app start alert stands down for a meeting whose banner macOS holds.
  it('is not doubled by a foreground tick inside the start window', async () => {
    await warnAt(4 * 60 * 1000);
    vi.setSystemTime(T0 - 30 * 1000);
    await tickNow();
    expect(startBanners()).toHaveLength(1);
    expect(notifyMock).toHaveBeenCalledTimes(2);
  });

  it('is withdrawn when a recording starts before the meeting', async () => {
    const { rerender } = await warnAt(4 * 60 * 1000);
    expect(cancelPendingMock).not.toHaveBeenCalled();
    recording = true;
    rerender(<CalendarAlerts />);
    expect(cancelPendingMock).toHaveBeenCalledWith('meeting-start-m-1');
    // …and a tick in the start window does not announce it in-app either.
    vi.setSystemTime(T0 - 30 * 1000);
    await tickNow();
    expect(startBanners()).toHaveLength(1);
  });

  it('follows the meeting when it is moved', async () => {
    await warnAt(4 * 60 * 1000);
    const moved = new Date(T0 + 2 * 60 * 1000).toISOString();
    getUpcomingMock.mockResolvedValue(meetingAt(moved));
    vi.setSystemTime(T0 - 3 * 60 * 1000);
    await tickNow();
    expect(startBanners()).toHaveLength(2);
    expect(startBanners()[1].deliverAtMs).toBe(T0 + 2 * 60 * 1000);
  });

  it('is withdrawn when the meeting leaves the list before it starts', async () => {
    await warnAt(4 * 60 * 1000);
    getUpcomingMock.mockResolvedValue([]);
    vi.setSystemTime(T0 - 3 * 60 * 1000);
    await tickNow();
    expect(cancelPendingMock).toHaveBeenCalledWith('meeting-start-m-1');
  });

  // A failed read is not an empty calendar.
  it('is kept when the calendar read fails', async () => {
    await warnAt(4 * 60 * 1000);
    getUpcomingMock.mockResolvedValue(null);
    vi.setSystemTime(T0 - 3 * 60 * 1000);
    await tickNow();
    expect(cancelPendingMock).not.toHaveBeenCalled();
  });

  it('is not scheduled while recording (the warning still fires)', async () => {
    recording = true;
    vi.setSystemTime(T0 - 4 * 60 * 1000);
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalledTimes(1));
    await settle();
    expect(startBanners()).toHaveLength(0);
  });

  // A meeting first seen inside the last minute takes the in-app path, delivered now.
  it('leaves a meeting first seen at T-30s to the in-app alert', async () => {
    vi.setSystemTime(T0 - 30 * 1000);
    render(<CalendarAlerts />);
    await waitFor(() => expect(notifyMock).toHaveBeenCalled());
    await settle();
    expect(startBanners()).toHaveLength(1);
    expect(startBanners()[0].deliverAtMs).toBeUndefined();
  });
});
