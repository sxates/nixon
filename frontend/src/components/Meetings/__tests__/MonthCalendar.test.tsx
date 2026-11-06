import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { MonthCalendar } from '@/components/Meetings/MonthCalendar';

/** A recorded meeting on a fixed local day, so assertions never depend on "now". */
const at = (day: number, hour: number, over: Record<string, unknown> = {}) => ({
  id: `m-${day}-${hour}`,
  title: `Meeting ${day}-${hour}`,
  createdAt: new Date(2026, 7, day, hour, 0, 0, 0).toISOString(), // local, August 2026
  hasSummary: false,
  ...over,
});

/** Grid cells are labelled "<Date.toDateString()>, N meetings". */
const cellFor = (day: number) =>
  screen.getByRole('gridcell', {
    name: new RegExp(`^${new Date(2026, 7, day).toDateString()},`),
  });

describe('MonthCalendar (specs/0054 W3)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.setSystemTime(new Date(2026, 7, 26, 12, 0, 0));
  });

  it('lets you select a day with no recordings and says so', async () => {
    // The owner reported empty days feeling inert — no hover, no response to a click.
    invokeMock.mockResolvedValue([at(12, 11)]);
    render(<MonthCalendar onOpenMeeting={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_get_meetings_in_range', expect.anything()));

    await userEvent.click(cellFor(13));

    expect(screen.getByText(/no meetings recorded this day/i)).toBeInTheDocument();
    expect(cellFor(13)).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the day’s true meeting count even when more meetings exist than chips', async () => {
    // Cells size to the window, so the chip list clips by an amount no counter can
    // predict. The badge must report the real total, never "chips rendered".
    invokeMock.mockResolvedValue([at(12, 9), at(12, 11), at(12, 13), at(12, 15), at(12, 17)]);
    render(<MonthCalendar onOpenMeeting={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    const cell = cellFor(12);
    expect(cell).toHaveTextContent('5');
    // The superseded affordance counted from the slice and under-reported once the
    // cell clipped; it must not come back.
    expect(cell).not.toHaveTextContent(/more/i);
  });

  it('omits the count badge for a day with a single recording', async () => {
    invokeMock.mockResolvedValue([at(12, 11)]);
    render(<MonthCalendar onOpenMeeting={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    // Only the day number — a "1" badge next to "12" would just be noise.
    expect(cellFor(12).textContent).not.toMatch(/\b1\b(?!2)/);
  });

  it('opens the meeting picked from the selected day', async () => {
    const onOpen = vi.fn();
    invokeMock.mockResolvedValue([at(12, 11, { title: 'Roadmap planning' })]);
    render(<MonthCalendar onOpenMeeting={onOpen} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    await userEvent.click(cellFor(12));
    await userEvent.click(screen.getByRole('button', { name: /roadmap planning/i }));

    expect(onOpen).toHaveBeenCalledWith('m-12-11');
  });

  it('surfaces a load failure instead of rendering an empty month', async () => {
    invokeMock.mockRejectedValue('db gone');
    vi.spyOn(console, 'error').mockImplementation(() => {});
    render(<MonthCalendar onOpenMeeting={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/could not load/i)).toBeInTheDocument());
  });
});

// With a calendar connected the Month view also shows the calendar's events, read through
// the same per-day `api_get_day_agenda` the Week view uses.
describe('MonthCalendar with a calendar connected', () => {
  const localIso = (day: number, hour: number) => new Date(2026, 7, day, hour, 0, 0, 0).toISOString();
  const event = (over: Record<string, unknown>) => ({
    id: 'evt',
    title: 'Event',
    startTime: localIso(28, 10),
    endTime: null,
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: null,
    status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    ...over,
  });
  const ctx = { now: new Date(2026, 7, 26, 12), isRecording: false, recordingThisId: null };
  const keyOf = (iso: string) => {
    const d = new Date(iso);
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  };

  /** Answers each day read with the events whose start falls on that local day. */
  function backend(recordings: unknown[], events: ReturnType<typeof event>[]) {
    invokeMock.mockImplementation(async (cmd: string, args?: { date?: string }) => {
      if (cmd === 'api_get_meetings_in_range') return recordings;
      if (cmd === 'api_get_day_agenda') {
        const items = events.filter((e) => keyOf(e.startTime as string) === args?.date);
        return { items, calendarSource: 'google' };
      }
      return [];
    });
  }

  beforeEach(() => {
    invokeMock.mockReset();
    vi.setSystemTime(new Date(2026, 7, 26, 12, 0, 0));
  });

  it('shows an upcoming event, and opening it does what Week does', async () => {
    backend([], [event({ id: 'evt-crit', title: 'Design crit' })]);
    const onSelect = vi.fn();
    render(<MonthCalendar onOpenMeeting={vi.fn()} events={{ ctx, onSelect }} />);

    await waitFor(() => expect(cellFor(28)).toHaveTextContent('Design crit'));
    await userEvent.click(cellFor(28));
    await userEvent.click(screen.getByRole('button', { name: /design crit/i }));
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 'evt-crit' }));
  });

  it('shows a meeting recorded from an event once, as the recording', async () => {
    backend(
      [at(12, 11, { title: 'Roadmap planning' })],
      [
        event({
          id: 'evt-roadmap',
          title: 'Roadmap planning (invite)',
          startTime: localIso(12, 11),
          meetingId: 'm-12-11',
          status: { recorded: true, transcribed: true, summarized: false, speakersIdentified: false },
        }),
      ],
    );
    render(<MonthCalendar onOpenMeeting={vi.fn()} events={{ ctx, onSelect: vi.fn() }} />);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_day_agenda', { date: '2026-08-12' }),
    );
    await waitFor(() => expect(cellFor(12)).toHaveTextContent('Roadmap planning'));
    expect(cellFor(12)).not.toHaveTextContent('(invite)');
    expect(cellFor(12)).toHaveAccessibleName(/, 1 meeting$/);
  });

  it('hides an event you dismissed', async () => {
    backend(
      [],
      [
        event({ id: 'evt-hidden', title: 'Hidden sync', startTime: localIso(29, 9), dismissed: true }),
        event({ id: 'evt-kept', title: 'Kept sync', startTime: localIso(29, 14) }),
      ],
    );
    render(<MonthCalendar onOpenMeeting={vi.fn()} events={{ ctx, onSelect: vi.fn() }} />);

    await waitFor(() => expect(cellFor(29)).toHaveTextContent('Kept sync'));
    expect(cellFor(29)).not.toHaveTextContent('Hidden sync');
  });

  it('reads no events without a calendar', async () => {
    backend([], [event({ title: 'Design crit' })]);
    render(<MonthCalendar onOpenMeeting={vi.fn()} />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_meetings_in_range', expect.anything()),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_day_agenda', expect.anything());
  });
});
