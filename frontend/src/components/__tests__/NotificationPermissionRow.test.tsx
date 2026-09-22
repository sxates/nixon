import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Owner feedback 2026-09-21 — "the 'meeting starts now - join & record' should be
// persistent, but all others transient." macOS gives no per-notification control (the
// Banner/Alert choice is one app-wide toggle; `.timeSensitive` needs an Apple-granted
// entitlement; specs/0068 already tried the Info.plist and category routes). So Nixon runs
// as Alerts and programmatically removes the ones that should not have stayed — which makes
// "set Nixon to Alerts" the one thing the user has to do, and therefore something Settings
// has to say.

const { capability, permission, openSettings, notifyMock } = vi.hoisted(() => ({
  capability: { supported: true, reason: null as string | null },
  permission: { value: 'authorized' as string },
  openSettings: vi.fn(),
  notifyMock: vi.fn().mockResolvedValue(true),
}));

vi.mock('@/lib/osNotification', () => ({
  CATEGORY_PLAIN: 'nixon.plain',
  getNotificationCapability: async () => capability,
  getNotificationPermission: async () => permission.value,
  notify: notifyMock,
  openNotificationSettings: openSettings,
  requestNotificationPermission: vi.fn().mockResolvedValue(true),
}));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

import { NotificationPermissionRow } from '@/components/NotificationPermissionRow';

beforeEach(() => {
  capability.supported = true;
  capability.reason = null;
  permission.value = 'authorized';
  openSettings.mockClear();
  notifyMock.mockClear();
});

describe('NotificationPermissionRow — the Alerts note', () => {
  it('tells an allowed user to pick Alerts, and says Nixon clears the rest', async () => {
    render(<NotificationPermissionRow />);
    const note = await screen.findByText(/set Nixon to/i);
    expect(note.textContent).toMatch(/Alerts/);
    expect(note.textContent).toMatch(/clears the rest by itself/i);
  });

  it('links straight into System Settings → Notifications', async () => {
    render(<NotificationPermissionRow />);
    await userEvent.click(await screen.findByRole('button', { name: 'Open Notifications' }));
    expect(openSettings).toHaveBeenCalled();
  });

  // Before permission exists, no banner can arrive at all — advice about their style would
  // be advice about something that cannot happen yet.
  it('stays quiet until notifications are actually allowed', async () => {
    permission.value = 'notDetermined';
    render(<NotificationPermissionRow />);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Allow…' })).toBeInTheDocument());
    expect(screen.queryByText(/set Nixon to/i)).not.toBeInTheDocument();
  });

  it('stays quiet when the build cannot deliver notifications at all', async () => {
    capability.supported = false;
    capability.reason = 'The dev build runs outside an .app bundle';
    render(<NotificationPermissionRow />);
    await waitFor(() =>
      expect(screen.getByText(/runs outside an .app bundle/)).toBeInTheDocument(),
    );
    expect(screen.queryByText(/set Nixon to/i)).not.toBeInTheDocument();
  });

  it('still offers the test notification alongside the note', async () => {
    render(<NotificationPermissionRow />);
    await userEvent.click(await screen.findByRole('button', { name: /Send a test/ }));
    expect(notifyMock).toHaveBeenCalled();
  });
});
