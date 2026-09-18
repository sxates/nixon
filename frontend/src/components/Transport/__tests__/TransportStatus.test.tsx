import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const { sidebar, transcripts, pushMock, pathname } = vi.hoisted(() => ({
  sidebar: { currentMeeting: null as { id: string; title: string } | null, activeRecordingMeetingId: null as string | null },
  transcripts: { meetingTitle: '' },
  pushMock: vi.fn(),
  pathname: { value: '/' },
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => sidebar }));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => transcripts }));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => ({ rms: 0.4, peak: 0.5, peakLatched: false }) }));
vi.mock('@/hooks/useMicGate', () => ({ useMicGate: () => false }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: pushMock }), usePathname: () => pathname.value }));

import { TransportStatus } from '@/components/Transport/TransportStatus';

beforeEach(() => {
  Object.assign(sidebar, { currentMeeting: null, activeRecordingMeetingId: null });
  Object.assign(transcripts, { meetingTitle: '' });
  pathname.value = '/';
  vi.clearAllMocks();
});

describe('TransportStatus title block', () => {
  it('while recording, is a button that routes back to /record', () => {
    Object.assign(sidebar, { activeRecordingMeetingId: 'm1' });
    Object.assign(transcripts, { meetingTitle: 'Pricing sync' });
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    const btn = screen.getByRole('button', { name: 'Back to the recording' });
    expect(btn.textContent).toContain('Pricing sync');
    fireEvent.click(btn);
    expect(pushMock).toHaveBeenCalledWith('/record');
  });

  it('is inert on /record — no button, title still shown', () => {
    pathname.value = '/record';
    Object.assign(sidebar, { activeRecordingMeetingId: 'm1' });
    Object.assign(transcripts, { meetingTitle: 'Pricing sync' });
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    expect(screen.queryByRole('button', { name: 'Back to the recording' })).toBeNull();
    expect(screen.getByText('Pricing sync')).toBeTruthy();
  });

  it('is inert when idle — nothing to navigate back to', () => {
    render(<TransportStatus phase="idle" elapsedSeconds={0} />);
    expect(screen.queryByRole('button', { name: 'Back to the recording' })).toBeNull();
    expect(screen.getByText('Deck ready')).toBeTruthy();
  });

  it('prefers the live transcript title over a stale sidebar title', () => {
    // The rename landed in TranscriptContext; SidebarProvider has not caught up yet.
    Object.assign(sidebar, { activeRecordingMeetingId: 'm1', currentMeeting: { id: 'm1', title: 'Old name' } });
    Object.assign(transcripts, { meetingTitle: 'New name' });
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    expect(screen.getByRole('button', { name: 'Back to the recording' }).textContent).toContain('New name');
  });

  it('falls back to the sidebar title, then to the literal', () => {
    Object.assign(sidebar, { activeRecordingMeetingId: 'm1', currentMeeting: { id: 'm1', title: 'Only name' } });
    Object.assign(transcripts, { meetingTitle: '+ New Call' }); // the unnamed-session placeholder
    const { unmount } = render(<TransportStatus phase="recording" elapsedSeconds={1} />);
    expect(screen.getByRole('button', { name: 'Back to the recording' }).textContent).toContain('Only name');
    unmount();

    Object.assign(sidebar, { activeRecordingMeetingId: 'm1', currentMeeting: null });
    render(<TransportStatus phase="recording" elapsedSeconds={1} />);
    expect(screen.getByRole('button', { name: 'Back to the recording' }).textContent).toContain('Recording');
  });
});
