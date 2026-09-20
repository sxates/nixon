import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// specs/0069 W4 (Ruling 2) — Today's "connect your calendar" nudge must gate on
// `calendarConnected` (EventKit OR Google, from `isAnyCalendarConnected`), not the
// EventKit-only `calendarStatus`. The bug this fixes: a user who connected Google and
// never granted EventKit access was told forever to connect a calendar they'd already
// connected, because the old gate only ever looked at EventKit's status.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ currentMeetingId: null, meetingTitle: null }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: vi.fn(), activeRecordingMeetingId: null }),
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ view: { pendingCount: 0, processing: false }, startNow: vi.fn() }),
}));
vi.mock('@/contexts/QueueOpenContext', () => ({
  useQueueOpen: () => ({ open: false, setOpen: vi.fn() }),
}));
vi.mock('framer-motion', () => {
  const passthrough = ({ children, ...props }: { children?: React.ReactNode }) => (
    <div {...props}>{children}</div>
  );
  return { motion: new Proxy({}, { get: () => passthrough }) };
});

const baseAgenda = {
  items: [],
  visibleItems: [],
  loaded: true,
  now: new Date('2026-09-20T15:00:00.000Z'),
  viewDate: '2026-09-20',
  viewMode: 'day' as const,
  viewIsToday: true,
  weekDays: [
    '2026-09-14',
    '2026-09-15',
    '2026-09-16',
    '2026-09-17',
    '2026-09-18',
    '2026-09-19',
    '2026-09-20',
  ],
  weekItems: [[]],
  weekLoaded: true,
  refresh: vi.fn(),
  goToDate: vi.fn(),
  goPrev: vi.fn(),
  goNext: vi.fn(),
  goToday: vi.fn(),
  switchMode: vi.fn(),
  openDay: vi.fn(),
  handleHide: vi.fn(),
};

const useDayAgendaMock = vi.fn();
vi.mock('@/hooks/useDayAgenda', () => ({ useDayAgenda: () => useDayAgendaMock() }));

import Home from '@/app/page';

beforeEach(() => {
  useDayAgendaMock.mockReset();
  try {
    window.localStorage.clear();
  } catch {
    /* not expected in jsdom */
  }
});

describe('Today calendar nudge gating (specs/0069 W4)', () => {
  it('does not render when Google is connected even though EventKit is notDetermined', () => {
    useDayAgendaMock.mockReturnValue({
      ...baseAgenda,
      calendarStatus: 'notDetermined',
      calendarConnected: true,
    });
    render(<Home />);
    expect(screen.queryByText('Connect your calendar to see your full day')).not.toBeInTheDocument();
  });

  it('renders when nothing is connected', () => {
    useDayAgendaMock.mockReturnValue({
      ...baseAgenda,
      calendarStatus: 'notDetermined',
      calendarConnected: false,
    });
    render(<Home />);
    expect(screen.getByText('Connect your calendar to see your full day')).toBeInTheDocument();
  });

  it('does not render before the check resolves (calendarConnected: null)', () => {
    useDayAgendaMock.mockReturnValue({
      ...baseAgenda,
      calendarStatus: 'notDetermined',
      calendarConnected: null,
    });
    render(<Home />);
    expect(screen.queryByText('Connect your calendar to see your full day')).not.toBeInTheDocument();
  });
});
