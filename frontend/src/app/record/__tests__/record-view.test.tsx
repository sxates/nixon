import React, { useEffect, useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The record screen's Transcript · My Notes · Agenda views (owner feedback, v0.18). Locks in:
// (a) Transcript is the default and the choice is remembered in localStorage (v0.18's
//     first-cut 'focus' reads as My Notes);
// (b) switching among all three never remounts the transcript or the notes column (the
//     notepad's editor, undo history and debounced autosave live there), it only restyles;
// (c) the To-Do sidebar exists only in My Notes / Agenda;
// (d) Agenda isn't offered before the meeting row exists;
// (e) a blocked localStorage still renders (and switches) the screen.

const mounts = { transcript: 0, rail: 0 };
const sidebar = { activeRecordingMeetingId: 'm-1' as string | null };

vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    setIsMeetingActive: vi.fn(),
    isContentInsetCollapsed: false,
    activeRecordingMeetingId: sidebar.activeRecordingMeetingId,
  }),
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
    RecordingHeader: ({
      view,
      onViewChange,
      activeRecordingMeetingId,
    }: {
      view: RecordView;
      onViewChange: (v: RecordView) => void;
      activeRecordingMeetingId: string | null;
    }) => <RecordViewTabs view={view} onViewChange={onViewChange} agendaAvailable={!!activeRecordingMeetingId} />,
  };
});

// Stateful stand-ins: a remount would reset the typed draft and bump the mount count.
vi.mock('@/app/_components/TranscriptPanel', () => ({
  TranscriptPanel: ({ topBar }: { topBar?: React.ReactNode }) => {
    useEffect(() => {
      mounts.transcript += 1;
    }, []);
    return (
      <div data-testid="transcript">
        transcript<div data-testid="transcript-strip">{topBar}</div>
      </div>
    );
  },
}));
vi.mock('@/components/Record/RecordMeetingControls', () => ({
  RecordMeetingControls: () => <div data-testid="meeting-controls" />,
}));
vi.mock('@/components/Record/RecordRail', () => ({
  RecordRail: ({ view, topBar }: { view: string; topBar?: React.ReactNode }) => {
    const [draft, setDraft] = useState('');
    useEffect(() => {
      mounts.rail += 1;
    }, []);
    return (
      <div data-testid="rail" data-view={view}>
        <textarea aria-label="notes" value={draft} onChange={(e) => setDraft(e.target.value)} />
        <div data-testid="rail-strip">{topBar}</div>
      </div>
    );
  },
}));
vi.mock('@/components/Record/RecordTodoRail', () => ({
  RecordTodoRail: ({ meetingId }: { meetingId: string | null }) => <div data-testid="todo-rail">{meetingId}</div>,
}));

import Home from '@/app/record/page';
import { RECORD_VIEW_KEY, type RecordView } from '@/lib/record-view';

beforeEach(() => {
  mounts.transcript = 0;
  mounts.rail = 0;
  sidebar.activeRecordingMeetingId = 'm-1';
  window.localStorage.clear();
});
afterEach(() => {
  vi.restoreAllMocks();
});

const tab = (name: RegExp) => screen.getByRole('tab', { name });

const go = (name: RegExp) => {
  fireEvent.mouseDown(tab(name));
  fireEvent.click(tab(name));
};

describe('record screen views', () => {
  it('offers Transcript · My Notes · Agenda (no Focus)', () => {
    render(<Home />);
    expect(screen.getByRole('tablist', { name: 'Recording view' })).toBeInTheDocument();
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['Transcript', 'My Notes', 'Agenda']);
  });

  it('mounts one meeting bar, heading whichever left column is on screen', () => {
    render(<Home />);
    expect(screen.getAllByTestId('meeting-controls')).toHaveLength(1);
    expect(screen.getByTestId('transcript-strip')).toContainElement(screen.getByTestId('meeting-controls'));

    for (const name of [/my notes/i, /agenda/i]) {
      go(name);
      expect(screen.getAllByTestId('meeting-controls')).toHaveLength(1);
      expect(screen.getByTestId('rail-strip')).toContainElement(screen.getByTestId('meeting-controls'));
      expect(screen.getByTestId('transcript-strip')).toBeEmptyDOMElement();
    }
  });

  it('defaults to Transcript: transcript shown, no To-Do sidebar', () => {
    render(<Home />);
    expect(tab(/transcript/i)).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('record-transcript-pane').className).not.toContain('invisible');
    expect(screen.getByTestId('rail')).toHaveAttribute('data-view', 'transcript');
    expect(screen.queryByTestId('todo-rail')).toBeNull();
  });

  it('Transcript → My Notes → Agenda → Transcript: nothing remounts, typed text survives, each is remembered', () => {
    render(<Home />);
    fireEvent.change(screen.getByLabelText('notes'), { target: { value: 'typed just before the switch' } });

    for (const [name, value] of [
      [/my notes/i, 'notes'],
      [/agenda/i, 'agenda'],
    ] as const) {
      go(name);
      expect(tab(name)).toHaveAttribute('aria-selected', 'true');
      const pane = screen.getByTestId('record-transcript-pane');
      expect(pane.className).toContain('invisible');
      expect(pane).toHaveAttribute('aria-hidden', 'true');
      expect(screen.getByTestId('transcript')).toBeInTheDocument();
      expect(screen.getByTestId('rail')).toHaveAttribute('data-view', value);
      expect(screen.getByTestId('todo-rail')).toHaveTextContent('m-1');
      expect(window.localStorage.getItem(RECORD_VIEW_KEY)).toBe(value);
      expect(screen.getByLabelText('notes')).toHaveValue('typed just before the switch');
    }

    go(/transcript/i);
    expect(screen.getByLabelText('notes')).toHaveValue('typed just before the switch');
    expect(mounts).toEqual({ transcript: 1, rail: 1 });
    expect(screen.queryByTestId('todo-rail')).toBeNull();
    expect(window.localStorage.getItem(RECORD_VIEW_KEY)).toBe('transcript');
  });

  it.each([
    ['notes', /my notes/i],
    ['agenda', /agenda/i],
    ['focus', /my notes/i],
  ] as const)('opens in the remembered view (stored %s)', async (stored, name) => {
    window.localStorage.setItem(RECORD_VIEW_KEY, stored);
    render(<Home />);
    await act(async () => {});
    expect(tab(name)).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('todo-rail')).toBeInTheDocument();
  });

  it('before the meeting row exists: no Agenda tab, and a stored Agenda shows My Notes', async () => {
    sidebar.activeRecordingMeetingId = null;
    window.localStorage.setItem(RECORD_VIEW_KEY, 'agenda');
    render(<Home />);
    await act(async () => {});
    expect(screen.queryByRole('tab', { name: /agenda/i })).toBeNull();
    expect(tab(/my notes/i)).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('rail')).toHaveAttribute('data-view', 'notes');
    // The choice itself is kept for when the row exists.
    expect(window.localStorage.getItem(RECORD_VIEW_KEY)).toBe('agenda');
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
    go(/my notes/i);
    expect(screen.getByTestId('todo-rail')).toBeInTheDocument();
  });
});
