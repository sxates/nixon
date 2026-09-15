import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

const { level } = vi.hoisted(() => ({
  level: { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } },
}));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => level }));
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
});

// specs/0057 Plan 2 Task 7 — the header is a control panel, not a second transport: no
// Pause/Stop buttons and no clock (the rail owns both).
describe('RecordingHeader', () => {
  it('idle: keeps the local-recording subtitle and carries no transport controls', () => {
    renderHeader(false);
    expect(screen.getByText('Recording locally on your Mac')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  it('recording: shows the engraved identity line and one VU per channel', () => {
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
    // 0.1.0 canvas feedback: no PEAK / MIC GATE lamps in the header any more.
    expect(screen.queryByText('Peak')).toBeNull();
    expect(screen.queryByText('Mic gate')).toBeNull();
    expect(screen.queryByRole('button', { name: /pause|resume|stop/i })).toBeNull();
  });

  it('back control is the unboxed chevron shared with meeting details', () => {
    renderHeader(false);
    const back = screen.getByRole('button', { name: 'Back to home' });
    expect(back.className).not.toMatch(/border-border|bg-card/);
  });
});
