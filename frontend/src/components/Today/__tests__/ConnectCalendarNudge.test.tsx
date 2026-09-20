import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0069 W4 (Ruling 2) — dismissing the nudge persists, so it doesn't come back on
// the next reload while still disconnected (the caller's `calendarConnected === false`
// gate is what makes it disappear for good once a calendar IS connected).

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import { ConnectCalendarNudge } from '@/components/Today/ConnectCalendarNudge';

beforeEach(() => {
  try {
    window.localStorage.clear();
  } catch {
    /* not expected in jsdom */
  }
});

describe('ConnectCalendarNudge dismiss (specs/0069 W4)', () => {
  it('hides on Dismiss and stays hidden across a remount', () => {
    const { unmount } = render(<ConnectCalendarNudge />);
    expect(screen.getByText(/connect your calendar/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    expect(screen.queryByText(/connect your calendar/i)).not.toBeInTheDocument();

    unmount();
    render(<ConnectCalendarNudge />);
    expect(screen.queryByText(/connect your calendar/i)).not.toBeInTheDocument();
  });
});
