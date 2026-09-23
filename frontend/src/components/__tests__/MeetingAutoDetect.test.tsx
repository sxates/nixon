import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';

// specs/0074 W5 — the detected-call prompt always reaches you. Pinned here: the OS banner
// goes out whether or not Nixon is focused, acting on either prompt takes the other down,
// and a banner that could not be sent is said out loud (once) instead of vanishing.

const h = vi.hoisted(() => ({
  toastMock: Object.assign(vi.fn(), { dismiss: vi.fn() }),
  notifyMock: vi.fn(),
  removeMock: vi.fn(),
  capabilityMock: vi.fn(),
  focusMock: vi.fn(),
  toggleMock: vi.fn(),
  pushMock: vi.fn(),
  stopMock: vi.fn(),
  pendingJoinMock: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('sonner', () => ({ toast: h.toastMock }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: h.pushMock }) }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, handler: (event: { payload: unknown }) => void) => {
    h.listeners.set(event, handler);
    return () => h.listeners.delete(event);
  },
}));
vi.mock('@/lib/osNotification', async () => {
  const actual = await vi.importActual<typeof import('@/lib/osNotification')>('@/lib/osNotification');
  return {
    ...actual,
    notify: h.notifyMock,
    removeNotification: h.removeMock,
    focusMainWindow: h.focusMock,
    getNotificationCapability: h.capabilityMock,
  };
});
vi.mock('@/lib/recording-stop', () => ({ requestFullRecordingStop: h.stopMock }));
vi.mock('@/lib/calendar', () => ({ peekPendingJoinMeeting: h.pendingJoinMock }));

let recording = false;
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: recording }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: h.toggleMock }),
}));

import MeetingAutoDetect from '@/components/MeetingAutoDetect';
import { CATEGORY_RECORD } from '@/lib/osNotification';

type ToastOptions = {
  id: string;
  description: string | ReactElement;
  action: { label: string; onClick: () => void };
  cancel: { label: string; onClick: () => void };
};

function emit(event: string, payload: unknown = { timestamp_ms: 111, kind: 'detected' }) {
  const handler = h.listeners.get(event);
  if (!handler) throw new Error(`no listener for ${event}`);
  handler({ payload });
}

/** Let the detection handler's awaits (notify, capability) settle. */
const settle = () => new Promise((r) => setTimeout(r, 0));

function lastToast(): [string, ToastOptions] {
  const calls = h.toastMock.mock.calls;
  return calls[calls.length - 1] as [string, ToastOptions];
}

function descriptionText(options: ToastOptions): string {
  if (typeof options.description === 'string') return options.description;
  const { container, unmount } = render(options.description);
  const text = container.textContent ?? '';
  unmount();
  return text;
}

beforeEach(() => {
  vi.clearAllMocks();
  h.listeners.clear();
  recording = false;
  h.notifyMock.mockResolvedValue(true);
  h.removeMock.mockResolvedValue(undefined);
  h.capabilityMock.mockResolvedValue({ supported: true, reason: null });
  h.pendingJoinMock.mockReturnValue(null);
});

describe('the OS banner always goes out', () => {
  it('notifies even while Nixon is focused', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    expect(h.notifyMock).toHaveBeenCalledTimes(1);
    const options = h.notifyMock.mock.calls[0][0];
    expect(options.category).toBe(CATEGORY_RECORD);
    expect(options.id).toBe('nixon-detected-111');
    // …and the in-app toast is there too, as the prompt that survives a refused permission.
    expect(h.toastMock).toHaveBeenCalled();
  });

  it('stays silent while a Join & Record is armed', async () => {
    h.pendingJoinMock.mockReturnValue({ id: 'e1' });
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    expect(h.notifyMock).not.toHaveBeenCalled();
    expect(h.toastMock).not.toHaveBeenCalled();
  });
});

describe('the copy names the app', () => {
  it('is Zoom when the payload does not say', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    expect(lastToast()[0]).toBe('Zoom call detected');
    expect(h.notifyMock.mock.calls[0][0].title).toBe('Zoom call detected');
  });

  it('uses the platform the payload names', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected', { timestamp_ms: 5, kind: 'detected', platform: 'teams' });
    await settle();
    expect(lastToast()[0]).toBe('Teams call detected');
    expect(h.notifyMock.mock.calls[0][0].title).toBe('Teams call detected');
  });
});

describe('acting on one prompt takes the other down', () => {
  it('Record in the toast removes the banner and starts recording once', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    lastToast()[1].action.onClick();
    expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111');
    expect(h.toggleMock).toHaveBeenCalledTimes(1);
  });

  it('Ignore in the toast removes the banner without recording', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    lastToast()[1].cancel.onClick();
    expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111');
    expect(h.toastMock.dismiss).toHaveBeenCalled();
    expect(h.toggleMock).not.toHaveBeenCalled();
  });

  it('Record on the banner dismisses the toast and starts recording', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    h.notifyMock.mock.calls[0][0].onRecord();
    expect(h.toastMock.dismiss).toHaveBeenCalledWith(lastToast()[1].id);
    expect(h.toggleMock).toHaveBeenCalledTimes(1);
  });

  it('an answer given while the banner was still being sent takes it down on arrival', async () => {
    let resolveNotify: (ok: boolean) => void = () => {};
    h.notifyMock.mockReturnValue(new Promise<boolean>((r) => (resolveNotify = r)));
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    lastToast()[1].cancel.onClick();
    resolveNotify(true);
    await settle();
    expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111');
  });

  it('a second call replaces the first prompt in place and takes its banner down', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    emit('zoom-meeting-detected', { timestamp_ms: 444, kind: 'detected' });
    await settle();
    expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111');
    // Dismissing then re-creating the same sonner id would take the new toast down too.
    expect(h.toastMock.dismiss).not.toHaveBeenCalled();
    expect(h.notifyMock.mock.calls[1][0].id).toBe('nixon-detected-444');
  });

  it('the meeting ending takes the banner down', async () => {
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    emit('zoom-meeting-ended', { timestamp_ms: 222, kind: 'ended' });
    expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111');
    expect(h.stopMock).not.toHaveBeenCalled(); // not recording: nothing to stop
  });

  it('a recording started some other way takes the banner down', async () => {
    const { rerender } = render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    recording = true;
    rerender(<MeetingAutoDetect />);
    await waitFor(() => expect(h.removeMock).toHaveBeenCalledWith('nixon-detected-111'));
  });
});

describe('a banner that could not be sent', () => {
  it('says notifications are off, with Enable leading to the permission row', async () => {
    h.notifyMock.mockResolvedValue(false);
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    const [, options] = lastToast();
    expect(descriptionText(options)).toContain('macOS notifications are off for Nixon');

    render(options.description as ReactElement);
    fireEvent.click(screen.getByRole('button', { name: 'Enable' }));
    expect(h.pushMock).toHaveBeenCalledWith('/settings?tab=general');
  });

  it('says so once per session', async () => {
    h.notifyMock.mockResolvedValue(false);
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    lastToast()[1].cancel.onClick();
    emit('zoom-meeting-detected', { timestamp_ms: 333, kind: 'detected' });
    await settle();
    expect(descriptionText(lastToast()[1])).toBe('Record this meeting?');
  });

  it('stays quiet in a build that cannot notify at all — there is nothing to enable', async () => {
    h.notifyMock.mockResolvedValue(false);
    h.capabilityMock.mockResolvedValue({ supported: false, reason: 'unbundled' });
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    await settle();
    expect(descriptionText(lastToast()[1])).toBe('Record this meeting?');
  });

  it('does not reopen a prompt the user already answered', async () => {
    h.notifyMock.mockResolvedValue(false);
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-detected');
    lastToast()[1].cancel.onClick();
    const before = h.toastMock.mock.calls.length;
    await settle();
    expect(h.toastMock.mock.calls.length).toBe(before);
  });
});

describe('the meeting ending while recording', () => {
  it('runs the full stop', async () => {
    recording = true;
    render(<MeetingAutoDetect />);
    emit('zoom-meeting-ended', { timestamp_ms: 9, kind: 'ended' });
    expect(h.stopMock).toHaveBeenCalledTimes(1);
  });
});
