import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0029 WS4.3 — per-meeting template persistence (the specs/0020 slice):
//  - the hook loads a persisted `meetings.template_id` for its target meeting;
//  - selecting a template persists via `api_set_meeting_template`;
//  - a selection made before the meeting row exists (recording just started,
//    `activeRecordingMeetingId` still null) is queued and flushed once the id arrives;
//  - a late-resolving persisted load never clobbers an explicit user selection;
//  - writes are guarded against the fabricated ids ('intro-call',
//    `meeting-<Date.now()>`) that must never be persisted against.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

// Mutable sidebar state so tests can vary the viewed-meeting fallback.
const sidebarState: { currentMeeting: { id: string; title: string } | null } = {
  currentMeeting: null,
};
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => sidebarState,
}));

import {
  useTemplates,
  isPersistableMeetingId,
  isSuggestableTitle,
  shouldConfirmTemplateChange,
  DEFAULT_TEMPLATE_ID,
} from '@/hooks/meeting-details/useTemplates';

const TEMPLATES = [
  { id: 'standard_meeting', name: 'Standard Meeting', description: 'default' },
  { id: 'daily_standup', name: 'Daily Standup', description: 'standup' },
];

const REAL_ID = 'meeting-123e4567-e89b-42d3-a456-426614174000';

beforeEach(() => {
  vi.clearAllMocks();
  sidebarState.currentMeeting = null;
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_list_templates':
        return Promise.resolve(TEMPLATES);
      case 'api_get_meeting_template':
        return Promise.resolve(null);
      case 'api_set_meeting_template':
        return Promise.resolve(null);
      default:
        return Promise.resolve(null);
    }
  });
});

describe('isPersistableMeetingId', () => {
  it('accepts real SQLite meeting ids and rejects fabricated/blank ones', () => {
    expect(isPersistableMeetingId(REAL_ID)).toBe(true);
    expect(isPersistableMeetingId('legacy-id')).toBe(true);
    expect(isPersistableMeetingId(null)).toBe(false);
    expect(isPersistableMeetingId(undefined)).toBe(false);
    expect(isPersistableMeetingId('')).toBe(false);
    expect(isPersistableMeetingId('   ')).toBe(false);
    expect(isPersistableMeetingId('intro-call')).toBe(false);
    // TranscriptContext's fabricated IndexedDB id: meeting-<Date.now()>.
    expect(isPersistableMeetingId(`meeting-${Date.now()}`)).toBe(false);
  });
});

describe('useTemplates — persistence (specs/0029 WS4.3)', () => {
  it('loads the persisted template for an explicit meeting id', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') return Promise.resolve('daily_standup');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID));

    await waitFor(() => expect(result.current.selectedTemplate).toBe('daily_standup'));
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_template', {
      meetingId: REAL_ID,
    });
  });

  it('keeps the default when no template is persisted (NULL)', async () => {
    const { result } = renderHook(() => useTemplates(REAL_ID));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_template', {
        meetingId: REAL_ID,
      }),
    );
    expect(result.current.selectedTemplate).toBe(DEFAULT_TEMPLATE_ID);
  });

  it('persists a selection against the explicit meeting id', async () => {
    const { result } = renderHook(() => useTemplates(REAL_ID));

    act(() => {
      result.current.handleTemplateSelection('daily_standup', 'Daily Standup');
    });

    expect(result.current.selectedTemplate).toBe('daily_standup');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_template', {
        meetingId: REAL_ID,
        templateId: 'daily_standup',
      }),
    );
  });

  it('queues a selection made before the row exists and flushes it when the id arrives', async () => {
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useTemplates(id),
      { initialProps: { id: null as string | null } },
    );

    act(() => {
      result.current.handleTemplateSelection('daily_standup', 'Daily Standup');
    });

    // No id yet: the choice applies locally but nothing is written.
    expect(result.current.selectedTemplate).toBe('daily_standup');
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_set_meeting_template',
      expect.anything(),
    );

    // Recording row created → activeRecordingMeetingId arrives → flush.
    rerender({ id: REAL_ID });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_template', {
        meetingId: REAL_ID,
        templateId: 'daily_standup',
      }),
    );
  });

  it('does not let a late persisted load clobber an explicit user selection', async () => {
    let resolveLoad: (value: string | null) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') {
        return new Promise((resolve) => {
          resolveLoad = resolve;
        });
      }
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID));

    // User picks while the load is still in flight…
    act(() => {
      result.current.handleTemplateSelection('daily_standup', 'Daily Standup');
    });
    // …then the stale persisted value resolves.
    await act(async () => {
      resolveLoad('standard_meeting');
    });

    expect(result.current.selectedTemplate).toBe('daily_standup');
  });

  it("falls back to the sidebar's viewed meeting when no id is passed (meeting-details)", async () => {
    sidebarState.currentMeeting = { id: REAL_ID, title: 'Weekly sync' };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') return Promise.resolve('daily_standup');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates());

    await waitFor(() => expect(result.current.selectedTemplate).toBe('daily_standup'));
  });

  it('never loads or saves against fabricated ids', async () => {
    sidebarState.currentMeeting = { id: 'intro-call', title: '+ New Call' };

    const { result } = renderHook(() => useTemplates());
    act(() => {
      result.current.handleTemplateSelection('daily_standup', 'Daily Standup');
    });

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_list_templates'));
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_get_meeting_template',
      expect.anything(),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_set_meeting_template',
      expect.anything(),
    );
  });
});

// specs/0020 task 10 — auto-select-by-title: with no persisted choice and no user
// selection, the most recent same-title meeting donates its template, which becomes
// the displayed selection AND is persisted immediately.
describe('useTemplates — auto-select by title (specs/0020)', () => {
  it('applies and persists a suggestion when nothing is persisted', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') return Promise.resolve(null);
      if (cmd === 'api_suggest_template_for_title') return Promise.resolve('daily_standup');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID, 'Weekly sync'));

    await waitFor(() => expect(result.current.selectedTemplate).toBe('daily_standup'));
    expect(invokeMock).toHaveBeenCalledWith('api_suggest_template_for_title', {
      title: 'Weekly sync',
      excludeMeetingId: REAL_ID,
    });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_meeting_template', {
        meetingId: REAL_ID,
        templateId: 'daily_standup',
      }),
    );
  });

  it('never suggests when a persisted value exists', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') return Promise.resolve('standard_meeting');
      if (cmd === 'api_suggest_template_for_title') return Promise.resolve('daily_standup');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID, 'Weekly sync'));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_template', {
        meetingId: REAL_ID,
      }),
    );
    expect(result.current.selectedTemplate).toBe('standard_meeting');
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_suggest_template_for_title',
      expect.anything(),
    );
  });

  it('ignores the suggestion path once the user has selected', async () => {
    let resolveLoad: (value: string | null) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      if (cmd === 'api_get_meeting_template') {
        return new Promise((resolve) => {
          resolveLoad = resolve;
        });
      }
      if (cmd === 'api_suggest_template_for_title') return Promise.resolve('standard_meeting');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID, 'Weekly sync'));

    // User picks while the persisted load is still in flight…
    act(() => {
      result.current.handleTemplateSelection('daily_standup', 'Daily Standup');
    });
    // …then the load resolves with "nothing persisted".
    await act(async () => {
      resolveLoad(null);
    });

    expect(result.current.selectedTemplate).toBe('daily_standup');
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_suggest_template_for_title',
      expect.anything(),
    );
  });

  it('skips the call for date-stamp titles ("Meeting <digit>…")', async () => {
    const { result } = renderHook(() => useTemplates(REAL_ID, 'Meeting 2026-07-02 10:00'));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_template', {
        meetingId: REAL_ID,
      }),
    );
    expect(result.current.selectedTemplate).toBe(DEFAULT_TEMPLATE_ID);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_suggest_template_for_title',
      expect.anything(),
    );
  });

  it('skips the call for empty/placeholder titles', async () => {
    const { result } = renderHook(() => useTemplates(REAL_ID, '   '));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_template', {
        meetingId: REAL_ID,
      }),
    );
    expect(result.current.selectedTemplate).toBe(DEFAULT_TEMPLATE_ID);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_suggest_template_for_title',
      expect.anything(),
    );
  });
});

describe('isSuggestableTitle', () => {
  it('accepts real titles and rejects empty or date-stamp ones', () => {
    expect(isSuggestableTitle('Weekly sync')).toBe(true);
    expect(isSuggestableTitle('  Design review  ')).toBe(true);
    expect(isSuggestableTitle('Meeting notes')).toBe(true); // "Meeting " + letter is fine
    expect(isSuggestableTitle(null)).toBe(false);
    expect(isSuggestableTitle(undefined)).toBe(false);
    expect(isSuggestableTitle('')).toBe(false);
    expect(isSuggestableTitle('   ')).toBe(false);
    expect(isSuggestableTitle('Meeting 2026-07-02 10:00')).toBe(false);
  });
});

// specs/0020 task 8 — pure confirm-before-regenerate decision (the dialog itself is
// exercised manually; this pins the trigger condition).
describe('shouldConfirmTemplateChange', () => {
  it('asks only for a DIFFERENT template when a summary already exists', () => {
    expect(shouldConfirmTemplateChange('daily_standup', 'standard_meeting', true)).toBe(true);
    // Same template → nothing to regenerate differently.
    expect(shouldConfirmTemplateChange('standard_meeting', 'standard_meeting', true)).toBe(false);
    // No summary yet → persist silently (today's behavior).
    expect(shouldConfirmTemplateChange('daily_standup', 'standard_meeting', false)).toBe(false);
  });
});

// specs/0053 W3 — Auto is the default structure for new meetings, pinned first
// in the picker. Meetings with an explicitly persisted template_id keep it.
describe('useTemplates — Auto default (specs/0053)', () => {
  it('defaults new meetings to Auto', () => {
    expect(DEFAULT_TEMPLATE_ID).toBe('auto');
  });

  it('pins Auto first in the picker, ahead of the built-ins', async () => {
    const { result } = renderHook(() => useTemplates());
    await waitFor(() => expect(result.current.availableTemplates.length).toBeGreaterThan(0));
    expect(result.current.availableTemplates[0].id).toBe('auto');
  });

  it('keeps an explicitly chosen fixed template instead of forcing Auto', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_meeting_template') return Promise.resolve('daily_standup');
      if (cmd === 'api_list_templates') return Promise.resolve(TEMPLATES);
      return Promise.resolve(null);
    });
    const { result } = renderHook(() => useTemplates('meeting-abc123'));
    await waitFor(() => expect(result.current.selectedTemplate).toBe('daily_standup'));
  });

  it('confirms before switching away from Auto on a meeting that already has a summary', () => {
    expect(shouldConfirmTemplateChange('daily_standup', 'auto', true)).toBe(true);
    expect(shouldConfirmTemplateChange('auto', 'auto', true)).toBe(false);
  });
});

// Hidden templates (specs/0020 follow-up): removed built-ins drop out of the
// picker list — unless one is the meeting's current selection, which must stay
// visible so the label/checkmark keep matching the summary being shown.
describe('useTemplates — hidden templates filter', () => {
  const WITH_HIDDEN = [
    { id: 'standard_meeting', name: 'Standard Meeting', description: 'std', hidden: false },
    { id: 'psychatric_session', name: 'Psychiatric Session', description: 'soap', hidden: true },
    { id: 'daily_standup', name: 'Daily Standup', description: 'standup', hidden: false },
  ];

  it('filters hidden templates out of availableTemplates', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(WITH_HIDDEN);
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID));

    await waitFor(() => expect(result.current.availableTemplates.length).toBe(3));
    expect(result.current.availableTemplates.map((t) => t.id)).toEqual([
      'auto',
      'standard_meeting',
      'daily_standup',
    ]);
  });

  it('keeps a hidden template visible while it is the persisted selection', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_list_templates') return Promise.resolve(WITH_HIDDEN);
      if (cmd === 'api_get_meeting_template') return Promise.resolve('psychatric_session');
      return Promise.resolve(null);
    });

    const { result } = renderHook(() => useTemplates(REAL_ID));

    await waitFor(() => expect(result.current.selectedTemplate).toBe('psychatric_session'));
    await waitFor(() =>
      expect(result.current.availableTemplates.map((t) => t.id)).toContain('psychatric_session'),
    );
  });
});
