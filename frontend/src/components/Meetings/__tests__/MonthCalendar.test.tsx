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
