import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

const { level, mode } = vi.hoisted(() => ({
  level: { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } },
  // `liveTranscription: null` makes ModeChip render nothing — the default for the tests that
  // don't care. The control-order test sets it so the chip is actually in the DOM.
  mode: { liveTranscription: null as boolean | null, onBattery: false },
}));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => level }));
vi.mock('@/hooks/useProcessingMode', () => ({ useProcessingMode: () => mode }));
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

const templates = { availableTemplates: [], selectedTemplate: null, handleTemplateSelection: vi.fn() } as unknown as
  React.ComponentProps<typeof RecordingHeader>['templates'];

function renderHeader(
  recording: boolean,
  overrides?: Partial<React.ComponentProps<typeof RecordingHeader>['templates']>,
) {
  return render(
    <RecordingHeader
      meetingTitle="Q3 planning"
      isRecordingActive={recording}
      activeRecordingMeetingId={recording ? 'm1' : null}
      titleEdit={titleEdit}
      templates={{ ...templates, ...overrides }}
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

  // Owner feedback 2026-09-21: "A better order: Participants, then Live, then Template" —
  // and the three used to be h-9 / h-9 / h-7, so the row stepped. Asserted on DOM order
  // rather than coordinates so it survives a restyle.
  it('per-meeting controls run Participants → Live → Template, all the same height', () => {
    mode.liveTranscription = true;
    renderHeader(true, {
      availableTemplates: [{ id: 't1', name: 'Standup', description: 'Short' }],
      selectedTemplate: 't1',
    } as unknown as React.ComponentProps<typeof RecordingHeader>['templates']);

    const participants = screen.getByRole('button', { name: 'Participants' });
    const live = screen.getByRole('button', { name: /Transcription mode/i });
    const template = screen.getByRole('button', { name: /Standup/ });

    // compareDocumentPosition: FOLLOWING (0x04) means the argument comes after the node.
    expect(participants.compareDocumentPosition(live) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy();
    expect(live.compareDocumentPosition(template) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy();

    expect(live.className).toMatch(/\bh-8\b/);
    expect(template.className).toMatch(/\bh-8\b/);
    expect(live.className).not.toMatch(/\bh-9\b/);
  });

  // The channel name used to be a `u-section-label` span UNDER each meter, costing the
  // header a text row per channel. It is now engraved inside the well.
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
    const back = screen.getByRole('button', { name: 'Back to home' });
    expect(back.className).not.toMatch(/border-border|bg-card/);
  });
});
