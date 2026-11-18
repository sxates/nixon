import React, { useEffect, useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The record screen's Transcript / Focus switch (owner feedback, v0.18). Locks in:
// (a) Transcript is the default and Focus is remembered in localStorage;
// (b) switching never remounts the transcript or the notes column (the notepad's
//     editor, undo history and debounced autosave live there), it only restyles them;
// (c) the To-Do rail exists only in Focus;
// (d) a blocked localStorage still renders (and switches) the screen.

const mounts = { transcript: 0, rail: 0 };

vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ setIsMeetingActive: vi.fn(), isContentInsetCollapsed: false, activeRecordingMeetingId: 'm-1' }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ status: 'recording', isStopping: false, isProcessing: false, isRecording: true }),
  RecordingStatus: { PROCESSING_TRANSCRIPTS: 'processing', SAVING: 'saving' },
}));
vi.mock('@/contexts/TranscriptContext', () => ({ useTranscripts: () => ({ meetingTitle: 'Weekly sync' }) }));
vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => ({ transcriptModelConfig: {} }) }));
vi.mock('@/app/_components/StatusOverlays', () => ({ StatusOverlays: () => null }));
vi.mock('@/app/_components/SettingsModal', () => ({ SettingsModals: () => null }));
vi.mock('@/components/TranscriptRecovery', () => ({ TranscriptRecovery: () => null }));
vi.mock('@/hooks/useModalState', () => ({
  useModalState: () => ({ modals: {}, messages: {}, showModal: vi.fn(), hideModal: vi.fn() }),
}));
vi.mock('@/hooks/useRecordingStateSync', () => ({ useRecordingStateSync: () => {} }));
vi.mock('@/hooks/useRecordingStart', () => ({ useRecordingStart: () => {} }));
vi.mock('@/hooks/useTranscriptionErrorListeners', () => ({ useTranscriptionErrorListeners: () => {} }));
vi.mock('@/hooks/useRecordingTitleEdit', () => ({ useRecordingTitleEdit: () => ({}), TITLE_PLACEHOLDER: '+ New Call' }));
vi.mock('@/hooks/useStartupRecovery', () => ({
  useStartupRecovery: () => ({
    showRecoveryDialog: false,
    recoverableMeetings: [],
    handleRecovery: vi.fn(),
    handleDialogClose: vi.fn(),
    deleteRecoverableMeeting: vi.fn(),
    loadMeetingTranscripts: vi.fn(),
  }),
}));
vi.mock('@/hooks/meeting-details/useSummaryPrompts', () => ({ useSummaryPrompts: () => ({}) }));

// The header's real switch, without the rest of the faceplate.
vi.mock('@/components/Record/RecordingHeader', async () => {
  const { RecordViewTabs } = await import('@/components/Record/RecordViewTabs');
  return {
    RecordingHeader: ({ view, onViewChange }: { view: 'transcript' | 'focus'; onViewChange: (v: 'transcript' | 'focus') => void }) => (
      <RecordViewTabs view={view} onViewChange={onViewChange} />
    ),
  };
});

// Stateful stand-ins: a remount would reset the typed draft and bump the mount count.
vi.mock('@/app/_components/TranscriptPanel', () => ({
  TranscriptPanel: () => {
    useEffect(() => {
      mounts.transcript += 1;
    }, []);
    return <div data-testid="transcript">transcript</div>;
  },
}));
vi.mock('@/components/Record/RecordRail', () => ({
  RecordRail: ({ view }: { view: string }) => {
    const [draft, setDraft] = useState('');
    useEffect(() => {
      mounts.rail += 1;
    }, []);
    return (
      <div data-testid="rail" data-view={view}>
        <textarea aria-label="notes" value={draft} onChange={(e) => setDraft(e.target.value)} />
      </div>
    );
  },
}));
vi.mock('@/components/Record/RecordTodoRail', () => ({
  RecordTodoRail: ({ meetingId }: { meetingId: string | null }) => <div data-testid="todo-rail">{meetingId}</div>,
}));

import Home from '@/app/record/page';
import { RECORD_VIEW_KEY } from '@/lib/record-view';

beforeEach(() => {
  mounts.transcript = 0;
  mounts.rail = 0;
  window.localStorage.clear();
});
afterEach(() => {
  vi.restoreAllMocks();
});

const tab = (name: RegExp) => screen.getByRole('tab', { name });

describe('record screen view switch', () => {
  it('defaults to Transcript: transcript shown, no To-Do rail', () => {
    render(<Home />);
    expect(tab(/transcript/i)).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('record-transcript-pane').className).not.toContain('invisible');
    expect(screen.getByTestId('rail')).toHaveAttribute('data-view', 'transcript');
    expect(screen.queryByTestId('todo-rail')).toBeNull();
  });

  it('Focus hides the transcript without unmounting anything, shows the To-Do rail, and is remembered', () => {
    render(<Home />);
    fireEvent.change(screen.getByLabelText('notes'), { target: { value: 'typed just before the switch' } });

    fireEvent.mouseDown(tab(/focus/i));
    fireEvent.click(tab(/focus/i));

    expect(tab(/focus/i)).toHaveAttribute('aria-selected', 'true');
    const pane = screen.getByTestId('record-transcript-pane');
    expect(pane.className).toContain('invisible');
    expect(pane).toHaveAttribute('aria-hidden', 'true');
    expect(screen.getByTestId('transcript')).toBeInTheDocument();
    expect(screen.getByTestId('rail')).toHaveAttribute('data-view', 'focus');
    expect(screen.getByTestId('todo-rail')).toHaveTextContent('m-1');
    expect(window.localStorage.getItem(RECORD_VIEW_KEY)).toBe('focus');

    // Back again: still the first mount of both, and the typed text is intact.
    fireEvent.mouseDown(tab(/transcript/i));
    fireEvent.click(tab(/transcript/i));
    expect(screen.getByLabelText('notes')).toHaveValue('typed just before the switch');
    expect(mounts).toEqual({ transcript: 1, rail: 1 });
    expect(screen.queryByTestId('todo-rail')).toBeNull();
    expect(window.localStorage.getItem(RECORD_VIEW_KEY)).toBe('transcript');
  });

  it('opens in the remembered view', async () => {
    window.localStorage.setItem(RECORD_VIEW_KEY, 'focus');
    render(<Home />);
    await act(async () => {});
    expect(tab(/focus/i)).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('todo-rail')).toBeInTheDocument();
  });

  it('ignores an unknown stored value, and still works when storage throws', async () => {
    window.localStorage.setItem(RECORD_VIEW_KEY, 'bogus');
    const { unmount } = render(<Home />);
    await act(async () => {});
    expect(tab(/transcript/i)).toHaveAttribute('aria-selected', 'true');
    unmount();

    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    render(<Home />);
    await act(async () => {});
    expect(tab(/transcript/i)).toHaveAttribute('aria-selected', 'true');
    fireEvent.mouseDown(tab(/focus/i));
    fireEvent.click(tab(/focus/i));
    expect(screen.getByTestId('todo-rail')).toBeInTheDocument();
  });
});
