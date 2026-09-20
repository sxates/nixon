import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0066, owner feedback on the first attempt. That version had two buttons: a Connect
// that asked EventKit directly, and a separate "Use Google". Both were wrong. Asking
// EventKit from a banner can only end, when the answer is no, in a toast telling you to go
// and find a setting — which is not an answer to "connect my calendar", it's a redirect
// with extra steps. And splitting the two sources made the card argue about a choice
// Settings already presents properly. One button, and it goes where the choice lives.

const { pushMock } = vi.hoisted(() => ({ pushMock: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));

import { ConnectCalendarCard, CALENDAR_SETTINGS_ROUTE } from '@/components/Calendar/ConnectCalendarCard';

beforeEach(() => vi.clearAllMocks());

describe('ConnectCalendarCard', () => {
  it('offers exactly one action, and it goes to the calendar settings', () => {
    render(<ConnectCalendarCard title="Connect your calendar" />);

    const buttons = screen.getAllByRole('button');
    expect(buttons).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(pushMock).toHaveBeenCalledWith(CALENDAR_SETTINGS_ROUTE);
  });

  it('routes to the Calendar section, not just the top of Settings', () => {
    // The anchor is the difference between "here is the setting" and "go and find it".
    expect(CALENDAR_SETTINGS_ROUTE).toBe('/settings?tab=general#calendar');
  });

  it('names both calendar sources', () => {
    render(<ConnectCalendarCard title="Connect your calendar" />);
    const copy = screen.getByText(/Connect your Mac/).textContent ?? '';
    expect(copy).toContain('Google');
    expect(copy).toContain('Mac');
  });

  it('never asks for calendar permission from the banner itself', () => {
    render(<ConnectCalendarCard title="Connect your calendar" />);
    // No "Connecting…" state to get stuck in, because nothing async happens here.
    expect(screen.queryByText(/connecting/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /google/i })).toBeNull();
  });

  it('shows Dismiss only where the caller offers one', () => {
    const { rerender } = render(<ConnectCalendarCard title="Connect your calendar" />);
    expect(screen.queryByRole('button', { name: 'Dismiss' })).toBeNull();

    const onDismiss = vi.fn();
    rerender(<ConnectCalendarCard title="Connect your calendar" onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(onDismiss).toHaveBeenCalled();
  });
});
