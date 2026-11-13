import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { FOCUS_QUICK_ADD_EVENT, QuickAddShortcut } from '../QuickAddShortcut';

const pushMock = vi.fn();
let pathname = '/meetings';
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock }),
  usePathname: () => pathname,
}));
let recording = false;
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: recording }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ activeRecordingMeetingId: recording ? 'm-live' : null }),
}));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ meetingTitle: 'Weekly sync' }),
}));
vi.mock('@/components/ActionItems/MentionInput', () => ({
  MentionInput: (p: { ariaLabel?: string }) => <input aria-label={p.ariaLabel} />,
}));

describe('QuickAddShortcut', () => {
  beforeEach(() => {
    pushMock.mockReset();
    recording = false;
  });

  it('⌘T opens the To-do page with the add field requested from another page', () => {
    pathname = '/meetings';
    render(<QuickAddShortcut />);
    fireEvent.keyDown(window, { key: 't', metaKey: true });
    expect(pushMock).toHaveBeenCalledWith('/tasks?tab=todo&add=1');
  });

  it('⌘T on the To-do page asks the page to focus the field instead of navigating', () => {
    pathname = '/tasks';
    const onFocus = vi.fn();
    window.addEventListener(FOCUS_QUICK_ADD_EVENT, onFocus);
    render(<QuickAddShortcut />);
    fireEvent.keyDown(window, { key: 't', metaKey: true });
    expect(onFocus).toHaveBeenCalledTimes(1);
    expect(pushMock).not.toHaveBeenCalled();
    window.removeEventListener(FOCUS_QUICK_ADD_EVENT, onFocus);
  });

  it('⌘⇧T opens the quick-add dialog in place, offering the live meeting while recording', async () => {
    pathname = '/record';
    recording = true;
    render(<QuickAddShortcut />);
    fireEvent.keyDown(window, { key: 'T', metaKey: true, shiftKey: true });
    expect(await screen.findByLabelText('Quick add to-do')).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: /Add to “Weekly sync” and its notes/ })).toBeChecked();
    expect(pushMock).not.toHaveBeenCalled();
  });

  it('⌘⇧T with no recording has no meeting option; plain t does nothing', async () => {
    render(<QuickAddShortcut />);
    fireEvent.keyDown(window, { key: 't' });
    expect(screen.queryByLabelText('Quick add to-do')).toBeNull();
    fireEvent.keyDown(window, { key: 'T', metaKey: true, shiftKey: true });
    expect(await screen.findByLabelText('Quick add to-do')).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).toBeNull();
  });
});
