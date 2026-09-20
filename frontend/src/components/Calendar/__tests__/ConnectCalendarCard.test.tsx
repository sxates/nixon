import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0066 W5 — every calendar affordance has to name both sources. Today's nudge and the
// Upcoming section each said "Nixon reads your macOS Calendar" and each offered exactly one
// button, which asks EventKit. Google Calendar has been a first-class source since
// specs/0032 and is the likelier one for most people, but it was reachable only by knowing
// to open Settings → General → Calendar. Two copies of that sentence is how it drifted in
// both places at once; these tests pin the single copy that replaced them.

const { pushMock } = vi.hoisted(() => ({ pushMock: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }) }));

import { ConnectCalendarCard } from '@/components/Calendar/ConnectCalendarCard';

beforeEach(() => vi.clearAllMocks());

describe('ConnectCalendarCard', () => {
  it('names Google Calendar alongside the macOS one', () => {
    render(<ConnectCalendarCard title="Connect your calendar" connecting={false} onConnect={vi.fn()} />);
    const copy = screen.getByText(/Nixon reads your/).textContent ?? '';
    expect(copy).toContain('Google');
    expect(copy).toContain('macOS');
  });

  it('routes to the calendar settings, where the Google connection lives', () => {
    render(<ConnectCalendarCard title="Connect your calendar" connecting={false} onConnect={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /use google/i }));
    expect(pushMock).toHaveBeenCalledWith('/settings?tab=general');
  });

  it('keeps Connect as the primary action, since EventKit is what it can grant from here', () => {
    const onConnect = vi.fn();
    render(<ConnectCalendarCard title="Connect your calendar" connecting={false} onConnect={onConnect} />);
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(onConnect).toHaveBeenCalled();
    expect(pushMock).not.toHaveBeenCalled();
  });

  it('shows Dismiss only where the caller offers one', () => {
    const { rerender } = render(
      <ConnectCalendarCard title="Connect your calendar" connecting={false} onConnect={vi.fn()} />,
    );
    expect(screen.queryByRole('button', { name: 'Dismiss' })).toBeNull();

    const onDismiss = vi.fn();
    rerender(
      <ConnectCalendarCard
        title="Connect your calendar"
        connecting={false}
        onConnect={vi.fn()}
        onDismiss={onDismiss}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it('disables Connect while a request is in flight', () => {
    render(<ConnectCalendarCard title="Connect your calendar" connecting onConnect={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Connecting…' })).toBeDisabled();
  });
});
