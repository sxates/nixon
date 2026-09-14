import React from 'react';
import { render, screen, act, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// specs/0029 WS5.1 — notes typed for one meeting leaked into another because saves
// re-resolved their target meeting id from the sidebar's VIEWED-meeting tracker at fire
// time. These tests lock the fixed semantics: every save (debounced autosave, unmount
// flush, id-switch flush) targets the meeting the current editor content was LOADED for.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock('sonner', () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

// The sidebar tracks the last VIEWED meeting — deliberately different from the meeting
// under test to prove saves never re-resolve from it.
const useSidebarMock = vi.fn();
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => useSidebarMock(),
}));

import { NotepadPanel } from '@/app/_components/NotepadPanel';

interface SaveCall {
  meetingId: string;
  notesMarkdown: string | null;
  notesJson: string | null;
}

/** api_save_meeting_notes payloads captured in call order. */
let saveCalls: SaveCall[];

function setupInvoke() {
  saveCalls = [];
  invokeMock.mockImplementation((command: unknown, args: unknown) => {
    if (command === 'api_get_meeting_notes') {
      // No stored notes; the panel starts blank for every meeting.
      return Promise.resolve(null);
    }
    if (command === 'api_save_meeting_notes') {
      saveCalls.push(args as SaveCall);
      return Promise.resolve(null);
    }
    return Promise.reject(new Error(`unexpected invoke: ${String(command)}`));
  });
}

/** Flush the async load effect + any timers due within `ms`. */
async function flush(ms = 0) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

function typeIntoEditor(html: string) {
  const editor = screen.getByRole('textbox');
  editor.innerHTML = html;
  fireEvent.input(editor);
}

beforeEach(() => {
  vi.useFakeTimers();
  setupInvoke();
  useSidebarMock.mockReturnValue({
    currentMeeting: { id: 'viewed-meeting-x', title: 'Old meeting X' },
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe('NotepadPanel save targeting (spec 0029 WS5.1)', () => {
  it('debounced autosave targets the pinned recording meeting, not the sidebar-viewed meeting', async () => {
    // The record screen pins the notepad to the live recording's id; the sidebar meanwhile
    // points at a previously viewed meeting (record Y → view X → back to /record).
    render(<NotepadPanel meetingId="recording-y" />);
    await flush();

    typeIntoEditor('<div>notes for the live recording</div>');
    await flush(1500); // past AUTOSAVE_DEBOUNCE_MS

    expect(saveCalls.length).toBeGreaterThan(0);
    for (const call of saveCalls) {
      expect(call.meetingId).toBe('recording-y');
    }
    expect(saveCalls.some((c) => c.notesJson?.includes('notes for the live recording'))).toBe(true);
  });

  it('unmount flush saves pending edits to the loaded meeting', async () => {
    const { unmount } = render(<NotepadPanel meetingId="recording-y" />);
    await flush();

    typeIntoEditor('<div>typed just before leaving</div>');
    // Unmount before the debounce fires — the flush must not drop or retarget the edit.
    unmount();
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(saveCalls).toHaveLength(1);
    expect(saveCalls[0].meetingId).toBe('recording-y');
    expect(saveCalls[0].notesJson).toContain('typed just before leaving');
  });

  it('flushes pending edits to the previous meeting before loading a new one (Y → X)', async () => {
    const { rerender } = render(<NotepadPanel meetingId="meeting-y" />);
    await flush();

    typeIntoEditor('<div>belongs to Y</div>');
    // Switch target before the debounce fires (e.g. navigating between meetings).
    rerender(<NotepadPanel meetingId="meeting-x" />);
    await flush(1500);

    // The pending edit was flushed under Y — never under X.
    const yCalls = saveCalls.filter((c) => c.meetingId === 'meeting-y');
    expect(yCalls).toHaveLength(1);
    expect(yCalls[0].notesJson).toContain('belongs to Y');
    expect(saveCalls.filter((c) => c.meetingId === 'meeting-x')).toHaveLength(0);

    // Edits made after the switch save under X.
    typeIntoEditor('<div>belongs to X</div>');
    await flush(1500);
    const xCalls = saveCalls.filter((c) => c.meetingId === 'meeting-x');
    expect(xCalls).toHaveLength(1);
    expect(xCalls[0].notesJson).toContain('belongs to X');
  });

  it('does not save while no meeting exists yet, then flushes typed content once the id appears', async () => {
    // Pre-recording window: no prop, sidebar holds only the "+ New Call" placeholder.
    useSidebarMock.mockReturnValue({ currentMeeting: { id: 'intro-call', title: '+ New Call' } });
    const { rerender } = render(<NotepadPanel />);
    await flush();

    typeIntoEditor('<div>typed before recording started</div>');
    await flush(1500);
    expect(saveCalls).toHaveLength(0); // nothing to save to yet

    // Recording starts: the record screen pins the new real id.
    rerender(<NotepadPanel meetingId="recording-z" />);
    await flush(1500);

    expect(saveCalls.length).toBeGreaterThan(0);
    for (const call of saveCalls) {
      expect(call.meetingId).toBe('recording-z');
    }
    expect(saveCalls.some((c) => c.notesJson?.includes('typed before recording started'))).toBe(true);
  });
});
