import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

const { level, gate } = vi.hoisted(() => ({
  level: { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } },
  gate: { muted: false },
}));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => level }));
vi.mock('@/hooks/useMicGate', () => ({ useMicGate: () => gate.muted }));
vi.mock('@/hooks/useProcessingMode', () => ({ useProcessingMode: () => ({ liveTranscription: null, onBattery: false }) }));
vi.mock('@/components/Participants/ParticipantsPopover', () => ({ ParticipantsPopover: () => null }));
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

function renderHeader(recording: boolean) {
  return render(
    <RecordingHeader
      meetingTitle="Q3 planning"
      isRecordingActive={recording}
      activeRecordingMeetingId={recording ? 'm1' : null}
      titleEdit={titleEdit}
      templates={templates}
    />,
  );
}

beforeEach(() => {
  Object.assign(level, { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } });
  gate.muted = false;
});

// specs/0057 Plan 2 Task 7 — the header is a control panel, not a second transport: no
// Pause/Stop buttons and no clock (the rail owns both).
describe('RecordingHeader', () => {
  it('idle: keeps the local-recording subtitle and carries no transport controls', () => {
    renderHeader(false);
    expect(screen.getByText('Recording locally on your Mac')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  it('recording: shows the engraved identity line, one VU per channel, and the lamp captions', () => {
    level.rms = 0.5;
    level.mic = { rms: 0.5, peak: 0.5 };
    level.sys = { rms: 0.02, peak: 0.02 };
    renderHeader(true);
    expect(screen.getByText('On the reel')).toBeTruthy();
    // specs/0057 §3.2: CH1 MIC and CH2 SYS read their own channel, not the mix.
    const meters = screen.getAllByRole('img', { name: /VU$/ });
    expect(meters.map((m) => m.getAttribute('aria-label'))).toEqual([
      expect.stringMatching(/^CH1 Mic level -?\d+ VU$/),
      expect.stringMatching(/^CH2 Sys level -?\d+ VU$/),
    ]);
    expect(screen.getByText('Peak')).toBeTruthy();
    expect(screen.getByText('Mic gate')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  it('lamps follow peak latch and the Zoom mute gate', () => {
    level.peakLatched = true;
    gate.muted = true;
    const { container } = renderHeader(true);
    const tones = Array.from(container.querySelectorAll('[data-tone]')).map((n) => n.getAttribute('data-tone'));
    expect(tones).toEqual(['red', 'amber']);
    // The lamps are decorative: the engraved captions beside them carry the meaning, so a
    // screen reader must not hear the state twice.
    expect(screen.queryByRole('img', { name: /peak|mic gate/i })).toBeNull();
  });
});
