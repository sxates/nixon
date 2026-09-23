import { describe, it, expect, vi, beforeEach } from 'vitest';

// specs/0068 — the routing this file is responsible for. The old implementation could not
// be tested at all, because the plugin calls it depended on do not exist on desktop; these
// cover the parts that now decide behaviour: the permission ladder, and which callback a
// press reaches.

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

import {
  ACTION_JOIN_AND_RECORD,
  ACTION_OPEN,
  ACTION_PREP,
  ACTION_RECORD,
  CATEGORY_MEETING,
  __resetNotificationStateForTests,
  ensureNotificationPermission,
  notify,
  removeNotification,
} from '@/lib/osNotification';

/** The handler `listen('notification-action', …)` was given. */
type ActionHandler = (event: { payload: Record<string, unknown> }) => void;
let handler: ActionHandler | null = null;

function backend({
  supported = true,
  status = 'authorized',
  requestGrants = true,
  deliverFails = false,
}: {
  supported?: boolean;
  status?: string;
  requestGrants?: boolean;
  deliverFails?: boolean;
} = {}) {
  invokeMock.mockImplementation(async (command: string) => {
    switch (command) {
      case 'notif_capability':
        return { supported, reason: supported ? null : 'not in a bundle' };
      case 'notif_authorization_status':
        return status;
      case 'notif_request_authorization':
        return requestGrants;
      case 'notif_deliver':
        if (deliverFails) throw new Error('delivery failed');
        return null;
      default:
        return null;
    }
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetNotificationStateForTests();
  handler = null;
  listenMock.mockImplementation(async (_event: string, cb: ActionHandler) => {
    handler = cb;
    return () => {};
  });
});

describe('permission', () => {
  it('does not ask when the build cannot deliver at all', async () => {
    backend({ supported: false });
    expect(await ensureNotificationPermission()).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith('notif_request_authorization');
  });

  it('asks macOS the first time, and only when it has not been asked', async () => {
    backend({ status: 'not_determined' });
    expect(await ensureNotificationPermission()).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('notif_request_authorization');
  });

  it('does not re-ask once allowed', async () => {
    backend({ status: 'authorized' });
    expect(await ensureNotificationPermission()).toBe(true);
    expect(invokeMock).not.toHaveBeenCalledWith('notif_request_authorization');
  });

  // macOS returns the stored answer without a dialog, so asking again would do nothing
  // except make the button look broken. The UI offers System Settings instead.
  it('does not re-ask once refused', async () => {
    backend({ status: 'denied' });
    expect(await ensureNotificationPermission()).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith('notif_request_authorization');
  });
});

describe('notify', () => {
  it('reports failure rather than throwing when permission is refused', async () => {
    backend({ status: 'denied' });
    expect(await notify({ title: 'T', body: 'B' })).toBe(false);
    expect(invokeMock).not.toHaveBeenCalledWith('notif_deliver', expect.anything());
  });

  it('reports failure when delivery throws, so the caller can fall back', async () => {
    backend({ deliverFails: true });
    expect(await notify({ title: 'T', body: 'B' })).toBe(false);
  });

  it('sends the category and the id the caller chose', async () => {
    backend();
    await notify({ title: 'T', body: 'B', category: CATEGORY_MEETING, id: 'meeting-42' });
    expect(invokeMock).toHaveBeenCalledWith('notif_deliver', {
      request: expect.objectContaining({ id: 'meeting-42', category: CATEGORY_MEETING }),
    });
  });
});

describe('a press comes back to the right callback', () => {
  async function sendAndPress(actionId: string) {
    backend();
    const pressed: string[] = [];
    await notify({
      title: 'T',
      body: 'B',
      category: CATEGORY_MEETING,
      id: 'n-1',
      onJoinAndRecord: () => pressed.push('joinAndRecord'),
      onRecord: () => pressed.push('record'),
      onPrep: () => pressed.push('prep'),
      onOpen: () => pressed.push('open'),
    });
    handler?.({ payload: { actionId, notificationId: 'n-1', userInfo: {} } });
    return pressed;
  }

  // macOS 26 shows a lone action as a button and hides two behind "Options", so a
  // category carries one; the ids the delegate can send are these three.
  it('routes Join & Record', async () => {
    expect(await sendAndPress(ACTION_JOIN_AND_RECORD)).toEqual(['joinAndRecord']);
  });

  it('routes Record', async () => {
    expect(await sendAndPress(ACTION_RECORD)).toEqual(['record']);
  });

  it('routes Prep', async () => {
    expect(await sendAndPress(ACTION_PREP)).toEqual(['prep']);
  });

  it('routes a body tap', async () => {
    expect(await sendAndPress(ACTION_OPEN)).toEqual(['open']);
  });

  it('drops a press for a banner it did not send', async () => {
    backend();
    const pressed: string[] = [];
    await notify({ title: 'T', body: 'B', id: 'mine', onOpen: () => pressed.push('open') });
    handler?.({ payload: { actionId: ACTION_OPEN, notificationId: 'someone-elses', userInfo: {} } });
    expect(pressed).toEqual([]);
  });

  // The banner is gone once pressed; a repeat would start a second recording.
  it('fires a callback at most once', async () => {
    backend();
    const pressed: string[] = [];
    await notify({
      title: 'T',
      body: 'B',
      id: 'once',
      category: CATEGORY_MEETING,
      onJoinAndRecord: () => pressed.push('joinAndRecord'),
    });
    handler?.({ payload: { actionId: ACTION_JOIN_AND_RECORD, notificationId: 'once', userInfo: {} } });
    handler?.({ payload: { actionId: ACTION_JOIN_AND_RECORD, notificationId: 'once', userInfo: {} } });
    expect(pressed).toEqual(['joinAndRecord']);
  });
});

// specs/0074 W5 — acting on the in-app prompt takes its banner twin down.
describe('removeNotification', () => {
  it('asks Rust to take the banner down and forgets its callbacks', async () => {
    backend();
    const pressed: string[] = [];
    await notify({ title: 'T', body: 'B', id: 'twin', onRecord: () => pressed.push('record') });
    await removeNotification('twin');
    expect(invokeMock).toHaveBeenCalledWith('notif_remove', { id: 'twin' });
    // A press that raced the removal must not start a second recording.
    handler?.({ payload: { actionId: ACTION_RECORD, notificationId: 'twin', userInfo: {} } });
    expect(pressed).toEqual([]);
  });

  it('never throws when the build cannot remove anything', async () => {
    invokeMock.mockRejectedValue(new Error('unbundled'));
    await expect(removeNotification('gone')).resolves.toBeUndefined();
  });
});
