import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const { level, mode } = vi.hoisted(() => ({
  level: { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } },
  // `liveTranscription: null` makes ModeChip render nothing — the default for the tests that
  // don't care. The control-order test sets it so the chip is actually in the DOM.
  mode: { liveTranscription: null as boolean | null, onBattery: false },
}));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => level }));
vi.mock('@/hooks/useProcessingMode', () => ({ useProcessingMode: () => mode }));
let transition: 'starting' | 'saving' | undefined = undefined;
vi.mock('@/hooks/useRecordEmptyPhase', () => ({ useRecordEmptyPhase: () => transition }));
vi.mock('@/components/Participants/ParticipantsPopover', () => ({
  ParticipantsPopover: () => <button type="button">Participants</button>,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import { RecordingHeader } from '@/components/Record/RecordingHeader';

const titleEdit = {
  isEditingTitle: false,
  titleDraft: '',
  setTitleDraft: vi.fn(),
  titleInputRef: { current: null },
  startEditingTitle: vi.fn(),
  commitTitleEdit: vi.fn(),
  cancelTitleEdit: vi.fn(),
} as unknown as React.ComponentProps<typeof RecordingHeader>['titleEdit'];

const promptsApi = {
  prompts: [],
  state: null,
  selectPrompt: vi.fn(),
  saveOneOff: vi.fn(),
  clearOneOff: vi.fn(),
  saveFollowup: vi.fn(),
} as unknown as React.ComponentProps<typeof RecordingHeader>['promptsApi'];

function renderHeader(
  recording: boolean,
  overrides?: Partial<React.ComponentProps<typeof RecordingHeader>['promptsApi']>,
) {
  return render(
    <RecordingHeader
      meetingTitle="Q3 planning"
      isRecordingActive={recording}
      activeRecordingMeetingId={recording ? 'm1' : null}
      titleEdit={titleEdit}
      promptsApi={{ ...promptsApi, ...overrides }}
    />,
  );
}

beforeEach(() => {
  Object.assign(level, { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } });
  Object.assign(mode, { liveTranscription: null, onBattery: false });
});

// specs/0057 Plan 2 Task 7 — the header is a control panel, not a second transport: no
// Pause/Stop buttons and no clock (the rail owns both).
describe('RecordingHeader', () => {
  it('idle: keeps the local-recording subtitle and carries no transport controls', () => {
    renderHeader(false);
    expect(screen.getByText('Recording locally on your Mac')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  it.each(['starting', 'saving'] as const)(
    'no idle subtitle while %s (owner feedback 2026-09-23: it flashed on the way in and out)',
    (phase) => {
      transition = phase;
      try {
        renderHeader(false);
        expect(screen.queryByText('Recording locally on your Mac')).toBeNull();
      } finally {
        transition = undefined;
      }
    },
  );

  it('recording: drops the subheads and shows one VU per channel', () => {
    level.rms = 0.5;
    level.mic = { rms: 0.5, peak: 0.5 };
    level.sys = { rms: 0.02, peak: 0.02 };
    renderHeader(true);
    // Owner feedback 2026-09-21: neither subhead survives a live recording. "On the reel"
    // duplicated the transport rail's own state line word for word, and the idle
    // "Recording locally on your Mac" is about what REC will do, so it has no business on
    // a header you can only reach mid-recording.
    expect(screen.queryByText('On the reel')).toBeNull();
    expect(screen.queryByText('Recording locally on your Mac')).toBeNull();
    // specs/0057 §3.2: CH1 MIC and CH2 SYS read their own channel, not the mix.
    const meters = screen.getAllByRole('img', { name: /VU$/ });
    expect(meters.map((m) => m.getAttribute('aria-label'))).toEqual([
      expect.stringMatching(/^CH1 Mic level -?\d+ VU$/),
      expect.stringMatching(/^CH2 Sys level -?\d+ VU$/),
    ]);
    // 0.1.0 canvas feedback: no PEAK / MIC GATE lamps in the header any more.
    expect(screen.queryByText('Peak')).toBeNull();
    expect(screen.queryByText('Mic gate')).toBeNull();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  // Owner feedback v0.18 (round 3): one row under the title carries the Transcript / Focus
  // tabs (left edge) and Participants → Prompt (right-aligned), so the title row has the
  // full width beside the meters; Pause transcript lives in the transcript's status strip.
  // Asserted on DOM order and containment rather than coordinates so it survives a restyle.
  it('tabs + Participants → Prompt share the row under the title; meters on the right; no Pause', () => {
    mode.liveTranscription = true;
    const onViewChange = vi.fn();
    render(
      <RecordingHeader
        meetingTitle="Q3 planning"
        isRecordingActive
        activeRecordingMeetingId="m1"
        titleEdit={titleEdit}
        promptsApi={{
          ...promptsApi,
          prompts: [{ id: 'p1', name: 'Standup' }],
          state: { source: 'meeting', prompt_id: 'p1', prompt_name: 'Standup', has_series: false },
        } as unknown as React.ComponentProps<typeof RecordingHeader>['promptsApi']}
        view="focus"
        onViewChange={onViewChange}
      />,
    );

    const tabs = screen.getByRole('tablist', { name: 'Recording view' });
    const title = screen.getByRole('heading', { level: 1 });
    const participants = screen.getByRole('button', { name: 'Participants' });
    const template = screen.getByRole('button', { name: /Standup/ });
    const meter = screen.getAllByRole('img', { name: /VU$/ })[0];
    const following = (a: Node, b: Node) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

    // Nothing but the title in the title row.
    const titleRow = title.closest('header') as HTMLElement;
    expect(titleRow.contains(tabs)).toBe(false);
    expect(titleRow.contains(participants)).toBe(false);
    // The tabs row: tabs, then Participants, then the prompt — all in one row element.
    const row = tabs.closest('div.mt-2\\.5') as HTMLElement;
    expect(row).not.toBeNull();
    expect(row.contains(participants) && row.contains(template)).toBe(true);
    expect(following(title, tabs) && following(tabs, participants) && following(participants, template)).toBe(true);
    // The meters come after the whole left column.
    expect(row.contains(meter)).toBe(false);
    expect(following(template, meter)).toBe(true);
    expect(screen.getByRole('tab', { name: 'Focus' })).toHaveAttribute('aria-selected', 'true');
    expect(template.className).toMatch(/\bh-8\b/);

    expect(screen.queryByRole('button', { name: /pause transcript|resume transcript/i })).toBeNull();
  });

  // The channel name used to be a `u-section-label` span UNDER each meter, costing the
  // header a text row per channel. It is now engraved inside the well.
  it('prompt picker selects a saved prompt and offers the one-off dialog', async () => {
    const selectPrompt = vi.fn();
    renderHeader(true, {
      prompts: [
        { id: 'p1', name: 'Standup' },
        { id: 'p2', name: 'Retro' },
      ],
      state: { source: 'meeting', prompt_id: 'p1', prompt_name: 'Standup', has_series: false },
      selectPrompt,
    } as unknown as React.ComponentProps<typeof RecordingHeader>['promptsApi']);

    await userEvent.click(screen.getByRole('button', { name: 'Summary prompt: Standup' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Retro' }));
    expect(selectPrompt).toHaveBeenCalledWith('p2');

    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: /custom for this Meeting/i }));
    expect(await screen.findByRole('dialog', { name: /custom prompt for this Meeting/i })).toBeInTheDocument();
  });

  it('each meter carries its channel label inside its own svg', () => {
    renderHeader(true);
    const meters = screen.getAllByRole('img', { name: /VU$/ });
    expect(meters).toHaveLength(2);
    expect(meters[0].querySelector('svg')?.textContent).toContain('CH1 MIC');
    expect(meters[1].querySelector('svg')?.textContent).toContain('CH2 SYS');
    // No stray label text outside the svg.
    expect(screen.queryByText('CH1 Mic')).toBeNull();
  });

  it('back control is the unboxed chevron shared with meeting details', () => {
    renderHeader(false);
    const back = screen.getByRole('button', { name: 'Back' });
    expect(back.className).not.toMatch(/border-border|bg-card/);
  });
});
