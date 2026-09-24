import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

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
  CALLBACK_TTL_MS,
  CATEGORY_MEETING,
  __resetNotificationStateForTests,
  cancelPending,
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

  // Exact args: a request that is not scheduled carries no `deliverAtMs` key at all.
  it('sends exactly the request Rust expects', async () => {
    backend();
    await notify({ title: 'T', body: 'B', category: CATEGORY_MEETING, id: 'meeting-42' });
    expect(invokeMock).toHaveBeenCalledWith('notif_deliver', {
      request: { id: 'meeting-42', title: 'T', body: 'B', category: CATEGORY_MEETING, userInfo: {} },
    });
  });

  // specs/0075 W2. Rust reads an i64, so a fractional value would reject the whole call.
  it('schedules with an integer deliverAtMs', async () => {
    backend();
    await notify({ title: 'T', body: 'B', id: 'meeting-start-1', deliverAtMs: 1_790_000_000_000.6 });
    expect(invokeMock).toHaveBeenCalledWith('notif_deliver', {
      request: {
        id: 'meeting-start-1',
        title: 'T',
        body: 'B',
        category: 'nixon.plain',
        userInfo: {},
        deliverAtMs: 1_790_000_000_001,
      },
    });
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

// specs/0075 W2 — a start banner scheduled at T-5 is pressed at T-0 or later.
describe('scheduled notifications', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const press = (notificationId: string) =>
    handler?.({ payload: { actionId: ACTION_JOIN_AND_RECORD, notificationId, userInfo: {} } });

  it('keeps its callbacks for the TTL counted from delivery, not from scheduling', async () => {
    vi.useFakeTimers();
    backend();
    const pressed: string[] = [];
    const deliverAtMs = Date.now() + 5 * 60 * 1000;
    await notify({
      title: 'T',
      body: 'B',
      id: 'meeting-start-1',
      category: CATEGORY_MEETING,
      deliverAtMs,
      onJoinAndRecord: () => pressed.push('joinAndRecord'),
    });
    // Past the TTL as counted from scheduling; still inside it as counted from delivery.
    vi.advanceTimersByTime(CALLBACK_TTL_MS + 60 * 1000);
    press('meeting-start-1');
    expect(pressed).toEqual(['joinAndRecord']);
  });

  it('still forgets them once the TTL after delivery has passed', async () => {
    vi.useFakeTimers();
    backend();
    const pressed: string[] = [];
    await notify({
      title: 'T',
      body: 'B',
      id: 'meeting-start-1',
      deliverAtMs: Date.now() + 5 * 60 * 1000,
      onJoinAndRecord: () => pressed.push('joinAndRecord'),
    });
    vi.advanceTimersByTime(5 * 60 * 1000 + CALLBACK_TTL_MS + 1);
    press('meeting-start-1');
    expect(pressed).toEqual([]);
  });

  // Re-delivering an id (a reschedule, the in-app start replacing a scheduled one) must not
  // let the first registration's expiry timer drop the second registration's callbacks.
  it('does not let an earlier registration expire a later one', async () => {
    vi.useFakeTimers();
    backend();
    const pressed: string[] = [];
    await notify({ title: 'T', body: 'B', id: 'n', onOpen: () => pressed.push('first') });
    vi.advanceTimersByTime(CALLBACK_TTL_MS - 1000);
    await notify({
      title: 'T',
      body: 'B',
      id: 'n',
      deliverAtMs: Date.now() + 10 * 60 * 1000,
      onOpen: () => pressed.push('second'),
    });
    vi.advanceTimersByTime(5000); // the first registration's timer would have fired here
    handler?.({ payload: { actionId: ACTION_OPEN, notificationId: 'n', userInfo: {} } });
    expect(pressed).toEqual(['second']);
  });
});

describe('cancelPending', () => {
  it('asks Rust to withdraw the request and forgets its callbacks', async () => {
    backend();
    const pressed: string[] = [];
    await notify({
      title: 'T',
      body: 'B',
      id: 'meeting-start-1',
      deliverAtMs: Date.now() + 60_000,
      onRecord: () => pressed.push('record'),
    });
    await cancelPending('meeting-start-1');
    expect(invokeMock).toHaveBeenCalledWith('notif_cancel_pending', { id: 'meeting-start-1' });
    handler?.({ payload: { actionId: ACTION_RECORD, notificationId: 'meeting-start-1', userInfo: {} } });
    expect(pressed).toEqual([]);
  });

  // An unbundled build rejects with the capability reason; callers never see it.
  it('never throws when the build cannot cancel anything', async () => {
    invokeMock.mockRejectedValue(new Error('unbundled'));
    await expect(cancelPending('gone')).resolves.toBeUndefined();
  });
});
