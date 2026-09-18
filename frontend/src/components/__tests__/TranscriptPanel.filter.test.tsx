import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import type { MeetingSpeaker, TranscriptSegmentData } from '@/types';

// specs/0061 W4 (task 3) — clicking a speaker filters the transcript to just their
// lines and shows a "Showing: <name>" chip with a clear affordance. This locks the
// TranscriptPanel half: given `speakerFilter`, only that speaker's segments render,
// and the chip's clear button calls back out to the owner (page-content), which
// clears the filter — mirrors TranscriptPanel.youOption.test.tsx's mocking style.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../MeetingDetails/TranscriptButtonGroup', () => ({
  TranscriptButtonGroup: () => null,
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ enqueueMeeting: vi.fn() }),
}));

import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';

const SEGMENTS: TranscriptSegmentData[] = [
  { id: 'seg-1', timestamp: 0, text: 'line from tomas', speaker: 'spk_1', speakerName: 'Tomas' },
  { id: 'seg-2', timestamp: 3, text: 'line from priya', speaker: 'spk_2', speakerName: 'Priya' },
  // A second raw diarization key consolidated under the same Person as spk_1 (ruling
  // R36) — the filter must include this line too when spk_1's group is selected.
  { id: 'seg-3', timestamp: 5, text: 'line from tomas alt key', speaker: 'spk_3', speakerName: 'Tomas' },
];

function makeSpeakersController(speakers: MeetingSpeaker[]): UseSpeakersReturn {
  return {
    speakers,
    attendees: [],
    people: [],
    suggestion: null,
    crossMeetingSuggestions: new Map(),
    dismissSuggestion: vi.fn(),
    isLoading: false,
    refresh: vi.fn().mockResolvedValue(undefined),
    renameSpeaker: vi.fn().mockResolvedValue(undefined),
    assignAttendee: vi.fn().mockResolvedValue(undefined),
    assignPerson: vi.fn().mockResolvedValue(undefined),
    mergeSpeakers: vi.fn().mockResolvedValue(undefined),
  };
}

const speakersController = makeSpeakersController([
  { speakerKey: 'spk_1', displayName: 'Tomas', isLocal: false },
  { speakerKey: 'spk_2', displayName: 'Priya', isLocal: false },
]);

function renderPanel(props: Partial<React.ComponentProps<typeof TranscriptPanel>> = {}) {
  return render(
    <TooltipProvider>
      <TranscriptPanel
        transcripts={[]}
        customPrompt=""
        onPromptChange={vi.fn()}
        onCopyTranscript={vi.fn()}
        onOpenMeetingFolder={vi.fn().mockResolvedValue(undefined)}
        isRecording={false}
        usePagination
        segments={SEGMENTS}
        meetingId="meeting-1"
        speakersController={speakersController}
        {...props}
      />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
});

describe('TranscriptPanel — speaker filter chip (specs/0061 W4 task 3)', () => {
  it('renders every speaker when no filter is set', () => {
    renderPanel();
    expect(screen.getByText('line from tomas')).toBeInTheDocument();
    expect(screen.getByText('line from priya')).toBeInTheDocument();
    expect(screen.queryByText(/^Showing:/)).not.toBeInTheDocument();
  });

  it('filters to just the selected speaker and shows the chip', () => {
    renderPanel({ speakerFilter: 'spk_1' });
    expect(screen.getByText('line from tomas')).toBeInTheDocument();
    expect(screen.queryByText('line from priya')).not.toBeInTheDocument();
    expect(screen.getByText('Showing: Tomas')).toBeInTheDocument();
  });

  it('calls onClearSpeakerFilter when the clear button is activated', () => {
    const onClear = vi.fn();
    renderPanel({ speakerFilter: 'spk_1', onClearSpeakerFilter: onClear });
    fireEvent.click(screen.getByRole('button', { name: 'Clear speaker filter' }));
    expect(onClear).toHaveBeenCalledTimes(1);
  });

  it('filters by every member key of a consolidated group, not just the primary (ruling R36)', () => {
    renderPanel({ speakerFilter: 'spk_1', speakerFilterKeys: ['spk_1', 'spk_3'] });
    expect(screen.getByText('line from tomas')).toBeInTheDocument();
    expect(screen.getByText('line from tomas alt key')).toBeInTheDocument();
    expect(screen.queryByText('line from priya')).not.toBeInTheDocument();
    // The chip's display name still resolves from the PRIMARY key.
    expect(screen.getByText('Showing: Tomas')).toBeInTheDocument();
  });

  it('falls back to [speakerFilter] as a single key when speakerFilterKeys is omitted', () => {
    renderPanel({ speakerFilter: 'spk_1' });
    expect(screen.getByText('line from tomas')).toBeInTheDocument();
    // spk_3 is a DIFFERENT key than the bare `speakerFilter` — excluded without
    // `speakerFilterKeys` naming it explicitly.
    expect(screen.queryByText('line from tomas alt key')).not.toBeInTheDocument();
  });

  it('does not hang or crash when the filter matches no loaded segments', () => {
    renderPanel({ speakerFilter: 'spk_nobody' });
    expect(screen.queryByText('line from tomas')).not.toBeInTheDocument();
    expect(screen.queryByText('line from priya')).not.toBeInTheDocument();
    // Falls back to the raw key when no MeetingSpeaker matches it.
    expect(screen.getByText('Showing: spk_nobody')).toBeInTheDocument();
  });

  // specs/0061 review, I3 — the cross-task defect: a meeting still marked `defer`
  // that genuinely has loaded transcripts must not flip into the "hasn't been
  // processed yet" / "Process now" empty state just because the active speaker
  // filter happens to match zero of the LOADED segments. "Process now" routes to
  // `replace_meeting_transcripts`, which deletes every transcript row (including any
  // user edits) with no warning — so misfiring this state on a filter miss is a
  // real data-loss risk, not just a display glitch.
  it('does not show the unprocessed/"Process now" state when a zero-match filter hides a deferred meeting\'s transcripts', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_meeting_processing_mode') return Promise.resolve('defer');
      if (cmd === 'api_meeting_audio_available') return Promise.resolve(true);
      return Promise.resolve(null);
    });

    renderPanel({ speakerFilter: 'spk_nobody' });

    // Wait for the processing-mode/audio-availability probes to resolve.
    await screen.findByText('Showing: spk_nobody');

    expect(screen.queryByText(/hasn't been processed yet/i)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Process now' })).not.toBeInTheDocument();
  });
});
