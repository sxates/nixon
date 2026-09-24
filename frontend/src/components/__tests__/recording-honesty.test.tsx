import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// specs/0071 W1 + W2 — the recording surfaces must not claim to be doing something they are
// not. The owner paused the transcript mid-meeting (by pressing a chip labelled "Live") and
// for 27.8 seconds nothing anywhere said the transcript had stopped: the chip now read
// "Deferred", the in-list indicator still pulsed "Listening…", and the rail still said "On
// the reel".
//
// These two surfaces are tested together because the contract is one sentence: whatever is
// true of the session is what the screen says.

const { level, mode, micMuted } = vi.hoisted(() => ({
  level: { rms: 0, peak: 0, peakLatched: false, mic: { rms: 0, peak: 0 }, sys: { rms: 0, peak: 0 } },
  mode: { liveTranscription: null as boolean | null, onBattery: false },
  micMuted: { value: false },
}));

vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => level }));
vi.mock('@/hooks/useProcessingMode', () => ({ useProcessingMode: () => mode }));
vi.mock('@/hooks/useRecordEmptyPhase', () => ({ useRecordEmptyPhase: () => undefined }));
vi.mock('@/hooks/useMicGate', () => ({ useMicGate: () => micMuted.value }));
vi.mock('@/hooks/useAutoScroll', () => ({
  useAutoScroll: () => ({ autoScroll: true, scrollToBottom: vi.fn() }),
}));
vi.mock('@/hooks/useTranscriptStreaming', () => ({
  useTranscriptStreaming: () => ({ streamingSegmentId: null, getDisplayText: (s: { text: string }) => s.text }),
}));
vi.mock('@/components/Participants/ParticipantsPopover', () => ({
  ParticipantsPopover: () => <button type="button">Participants</button>,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn() }),
  usePathname: () => '/record',
}));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ meetingTitle: 'Q3 planning' }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ currentMeeting: null, activeRecordingMeetingId: 'm1', isCollapsed: false }),
}));

import { RecordingHeader } from '@/components/Record/RecordingHeader';
import { TransportStatus } from '@/components/Transport/TransportStatus';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { TranscriptSegmentData } from '@/types';

const titleEdit = {
  isEditingTitle: false,
  titleDraft: '',
  setTitleDraft: vi.fn(),
  titleInputRef: { current: null },
  startEditingTitle: vi.fn(),
  commitTitleEdit: vi.fn(),
  cancelTitleEdit: vi.fn(),
} as unknown as React.ComponentProps<typeof RecordingHeader>['titleEdit'];

const templates = {
  availableTemplates: [],
  selectedTemplate: null,
  handleTemplateSelection: vi.fn(),
} as unknown as React.ComponentProps<typeof RecordingHeader>['templates'];

const SEGMENTS: TranscriptSegmentData[] = [
  { id: 's1', timestamp: 0, text: 'first line', speaker: 'spk_0', speakerName: 'You' },
];

function renderHeader() {
  return render(
    <RecordingHeader
      meetingTitle="Q3 planning"
      isRecordingActive
      activeRecordingMeetingId="m1"
      titleEdit={titleEdit}
      templates={templates}
    />,
  );
}

function renderTranscript(liveTranscription?: boolean) {
  return render(
    <TooltipProvider>
      <VirtualizedTranscriptView
        segments={SEGMENTS}
        isRecording
        disableAutoScroll
        {...(liveTranscription === undefined ? {} : { liveTranscription })}
      />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  Object.assign(mode, { liveTranscription: null, onBattery: false });
  micMuted.value = false;
});

describe('W1 — the chip is labelled with the action', () => {
  it('offers to pause while the transcript is running', () => {
    mode.liveTranscription = true;
    renderHeader();
    const chip = screen.getByRole('button', { name: 'Pause transcript' });
    expect(chip).toBeInTheDocument();
    // The old labels were the session's own state; a button may not be named that.
    expect(screen.queryByRole('button', { name: 'Live' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Deferred' })).toBeNull();
  });

  it('offers to resume while the transcript is paused', () => {
    mode.liveTranscription = false;
    renderHeader();
    expect(screen.getByRole('button', { name: 'Resume transcript' })).toBeInTheDocument();
  });

  // The accessible name must match the visible one — the old aria-label announced the mode
  // and then the switch, which is the same trap in longer form.
  it('reads the same to a screen reader as to an eye', () => {
    mode.liveTranscription = true;
    renderHeader();
    const chip = screen.getByRole('button', { name: 'Pause transcript' });
    expect(chip.getAttribute('aria-label')).toBe('Pause transcript');
    expect(chip.textContent).toContain('Pause transcript');
  });
});

describe('W1 — the rail carries the state', () => {
  it('says the transcript is paused while recording with live transcription off', () => {
    mode.liveTranscription = false;
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    expect(screen.getByText('Transcript paused')).toBeInTheDocument();
    expect(screen.queryByText('On the reel')).toBeNull();
  });

  it('says "On the reel" while the transcript is running', () => {
    mode.liveTranscription = true;
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    expect(screen.getByText('On the reel')).toBeInTheDocument();
    expect(screen.queryByText('Transcript paused')).toBeNull();
  });

  // Both of these are about the AUDIO, and audio beats transcript: a paused recording is not
  // transcribing either, and a muted mic is the more surprising of the two facts.
  it('lets "On hold" win over a paused transcript', () => {
    mode.liveTranscription = false;
    render(<TransportStatus phase="paused" elapsedSeconds={12} />);
    expect(screen.getByText('On hold')).toBeInTheDocument();
    expect(screen.queryByText('Transcript paused')).toBeNull();
  });

  it('lets "Mic muted" win over a paused transcript', () => {
    mode.liveTranscription = false;
    micMuted.value = true;
    render(<TransportStatus phase="recording" elapsedSeconds={12} />);
    expect(screen.getByText('Mic muted')).toBeInTheDocument();
    expect(screen.queryByText('Transcript paused')).toBeNull();
  });

  it('says nothing about the transcript when no recording is in progress', () => {
    mode.liveTranscription = false;
    render(<TransportStatus phase="idle" elapsedSeconds={0} />);
    expect(screen.getByText('Nothing on the reel')).toBeInTheDocument();
    expect(screen.queryByText('Transcript paused')).toBeNull();
  });
});

describe('W2 — the in-list indicator tells the truth', () => {
  it('says the transcript is paused AND that audio is still recording', () => {
    renderTranscript(false);
    // The second clause is the point: the fear a paused transcript creates is that the
    // recording stopped, and it had not.
    expect(
      screen.getByText('Transcript paused — audio is still recording'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Listening/)).toBeNull();
  });

  it('still says "Listening..." while live', () => {
    renderTranscript(true);
    expect(screen.getByText('Listening...')).toBeInTheDocument();
    expect(screen.queryByText(/Transcript paused/)).toBeNull();
  });

  // Defaulted true so meeting-details and every other caller are untouched.
  it('defaults to listening when the prop is absent', () => {
    renderTranscript(undefined);
    expect(screen.getByText('Listening...')).toBeInTheDocument();
  });
});
