import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

// specs/0061 W1 Task 3 — the final, skippable onboarding step offering both
// Google Calendar and macOS Calendar. Renders whichever Google connect state
// `useGoogleCalendarConnect` reports (mocked here — its own behavior is
// covered by useGoogleCalendarConnect.test.ts) and the EventKit row via
// `@/lib/calendar`. "Finish Setup" and "Skip for now" both complete
// onboarding then reload the app.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));

const connectMock = vi.fn().mockResolvedValue({ ok: true, email: 'user@halden.example' });
const useGoogleCalendarConnectMock = vi.fn();
vi.mock('@/hooks/useGoogleCalendarConnect', () => ({
  useGoogleCalendarConnect: () => useGoogleCalendarConnectMock(),
}));

const requestCalendarAccessMock = vi.fn().mockResolvedValue(true);
const getCalendarAccessStatusMock = vi.fn().mockResolvedValue('notDetermined');
vi.mock('@/lib/calendar', () => ({
  requestCalendarAccess: (...args: unknown[]) => requestCalendarAccessMock(...args),
  getCalendarAccessStatus: (...args: unknown[]) => getCalendarAccessStatusMock(...args),
}));

const completeOnboardingMock = vi.fn().mockResolvedValue(undefined);
vi.mock('@/contexts/OnboardingContext', () => ({
  useOnboarding: () => ({ completeOnboarding: completeOnboardingMock }),
}));

import { CalendarStep } from '@/components/onboarding/steps/CalendarStep';

function mockGoogleHook(overrides: Partial<ReturnType<typeof useGoogleCalendarConnectMock>> = {}) {
  useGoogleCalendarConnectMock.mockReturnValue({
    connecting: false,
    connect: connectMock,
    status: { configured: true, connected: false },
    refresh: vi.fn(),
    ...overrides,
  });
}

const reloadMock = vi.fn();

beforeEach(() => {
  connectMock.mockClear();
  completeOnboardingMock.mockClear();
  requestCalendarAccessMock.mockClear();
  getCalendarAccessStatusMock.mockClear();
  getCalendarAccessStatusMock.mockResolvedValue('notDetermined');
  reloadMock.mockClear();
  mockGoogleHook();
  Object.defineProperty(window, 'location', {
    configurable: true,
    value: { ...window.location, reload: reloadMock },
  });
});

describe('CalendarStep', () => {
  it('renders both rows when Google Calendar is configured', async () => {
    render(<CalendarStep />);
    await screen.findByText('macOS Calendar');
    expect(screen.getByText('Google Calendar')).toBeInTheDocument();
    expect(screen.getByText('macOS Calendar')).toBeInTheDocument();
  });

  it('renders only the macOS Calendar row when Google Calendar is not configured', async () => {
    mockGoogleHook({ status: { configured: false, connected: false } });
    render(<CalendarStep />);
    await screen.findByText('macOS Calendar');
    expect(screen.queryByText('Google Calendar')).toBeNull();
    expect(screen.getByText('macOS Calendar')).toBeInTheDocument();
  });

  it("the Google row's button calls connect()", async () => {
    render(<CalendarStep />);
    const googleRow = await screen.findByRole('group', { name: 'Google Calendar' });
    fireEvent.click(within(googleRow).getByRole('button'));
    expect(connectMock).toHaveBeenCalledTimes(1);
  });

  it('"Finish Setup" completes onboarding then reloads', async () => {
    render(<CalendarStep />);
    fireEvent.click(await screen.findByRole('button', { name: 'Finish Setup' }));

    await waitFor(() => expect(completeOnboardingMock).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(reloadMock).toHaveBeenCalledTimes(1));
  });

  it('"Skip for now" completes onboarding then reloads', async () => {
    render(<CalendarStep />);
    fireEvent.click(await screen.findByText('Skip for now'));

    await waitFor(() => expect(completeOnboardingMock).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(reloadMock).toHaveBeenCalledTimes(1));
  });

  it('uses the exact step copy', async () => {
    render(<CalendarStep />);
    expect(
      await screen.findByText('Connect a calendar (optional)'),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Nixon can show today's meetings and prepare briefs from your calendar.",
      ),
    ).toBeInTheDocument();
  });
});
