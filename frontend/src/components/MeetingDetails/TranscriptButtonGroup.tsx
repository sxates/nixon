"use client";

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Loader2, MoreHorizontal } from 'lucide-react';
import { RetranscribeDialog } from './RetranscribeDialog';
import { AudioSetupSubmenu } from './AudioSetupSubmenu';
import type {
  AudioSetupOverride,
  AudioSetupStartResult,
  MeetingAudioSetup,
} from '@/hooks/useAudioSetup';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useDiarization } from '@/hooks/useDiarization';
import { SPARSE_TRANSCRIPT_SEGMENTS } from '@/lib/deferred-transcription';
import { isMeetingInFlight } from '@/lib/deferred-backlog';
import {
  AUDIO_FAILED_NOTE,
  audioGoneTitle,
  canTranscribe,
  useMeetingAudioStatus,
} from '@/hooks/useMeetingAudioStatus';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
  /** The meeting's `origin` (spec 0015). Imported audio has no separate mic channel, so
   *  "Who was on the mic?" is hidden for it (specs/0078). Absent = recorded. */
  meetingOrigin?: string | null;
  /** "Who was on the mic?" state, owned by the speakers controller (`useSpeakers`) so the
   *  submenu and the "This is me" actions read one copy. */
  audioSetup?: MeetingAudioSetup | null;
  /** Store an override and start the re-run. Absent = the submenu is not offered. */
  onSetAudioSetup?: (setup: AudioSetupOverride) => Promise<AudioSetupStartResult>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  meetingOrigin,
  audioSetup = null,
  onSetAudioSetup,
}: TranscriptButtonGroupProps) {
  const { view, enqueueMeeting } = useBacklog();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  // 1.10 feedback: is this meeting marked deferred (recorded in low-power /
  // record-only mode, awaiting processing)? Probed once per meeting; null =
  // no marker / probe failed → the classic affordances below are unchanged.
  const [processingMode, setProcessingMode] = useState<string | null>(null);
  useEffect(() => {
    setProcessingMode(null);
    if (!meetingId) return;
    let cancelled = false;
    invoke<string | null>('api_get_meeting_processing_mode', { meetingId })
      .then((mode) => {
        if (!cancelled) setProcessingMode(mode);
      })
      .catch((error) => {
        console.warn('Failed to check meeting processing mode:', error);
      });
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  // What audio does this meeting still have? (specs/0072 W3). Retention deletes only
  // media files, so a meeting can keep its transcript with no audio. Each action is
  // gated on the audio IT needs: "Identify speakers" reads the system channel, Enhance /
  // Transcribe now read the mix (or channels Rust mixes). null = unknown — don't gate on
  // unknown, the flows' own errors still backstop.
  const audio = useMeetingAudioStatus(meetingId);
  const audioAvailable = audio ? canTranscribe(audio) : null;
  const speakersGoneTitle = audioGoneTitle(audio, 'channels');
  const transcribeGoneTitle = audioGoneTitle(audio, 'transcribe');
  const identifyFailed = audio?.state === 'failed';

  // specs/0029 WS7.2: a record-only meeting (live transcription off) has audio on
  // disk but no/sparse transcript rows — surface a first-class "Transcribe now"
  // affordance (reusing the retranscription engine/dialog) instead of hiding
  // transcription behind the beta "Enhance" button. Requires POSITIVE audio
  // availability so we never offer a button that must fail.
  const needsFirstTranscription =
    !!meetingId &&
    !!meetingFolderPath &&
    audioAvailable === true &&
    transcriptCount < SPARSE_TRANSCRIPT_SEGMENTS;

  // 1.10 feedback: a deferred meeting's manual trigger runs the FULL pipeline
  // (transcribe → diarize → summarize, then clears the defer marker) via the
  // same window event the backlog's immediate path handles — not the bare
  // retranscribe dialog, which would leave the marker set and the meeting
  // re-processed on the next return to AC power. Replaces "Transcribe now".
  const isDeferred = processingMode === 'defer';
  const offerProcessNow = isDeferred && !!meetingId && audioAvailable === true;

  const backlogStatus =
    meetingId ? (view.items.find((i) => i.meeting.id === meetingId)?.status ?? null) : null;
  // Shared with the meeting-details auto-summary gate (spec 0051 final review) so the
  // "is the backlog already on it?" question has exactly one answer app-wide.
  const isProcessingThis = isMeetingInFlight(view.items, meetingId);

  const handleProcessNow = useCallback(() => {
    if (!meetingId) return;
    enqueueMeeting(meetingId, { force: true });
  }, [meetingId, enqueueMeeting]);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  // Speaker diarization (specs/0010, P1-C). Re-fetch the transcript when a pass
  // completes so the new speaker labels appear.
  const {
    isRunning: isDiarizing,
    stage: diarizationStage,
    progressPct: diarizationPct,
    downloadProgress,
    identifySpeakers,
  } = useDiarization({
    meetingId,
    onComplete: onRefetchTranscripts,
  });

  // Stage text with an optional real percentage (e.g. "diarizing 42%"). Treat 0% as
  // indeterminate (specs/0024 WS4.1): the diarization run can sit at a reported 0% for a
  // while (sherpa emits progress only once its chunk loop advances), and a static "0%" reads
  // as hung. Show the bare stage (the spinner conveys activity) until progress actually moves.
  const hasRealProgress = diarizationPct !== null && diarizationPct > 0;
  const diarizationStatus =
    diarizationStage && hasRealProgress
      ? `${diarizationStage} ${diarizationPct}%`
      : diarizationStage;

  const handleIdentifySpeakers = useCallback(() => {
    void identifySpeakers();
  }, [identifySpeakers]);

  // "Who was on the mic?" (specs/0078). Only for a recording of our own whose channels
  // are still on disk: an import is one mixed file, and a notes-only meeting has no
  // audio (and no transcript tab) at all. Choosing re-runs the pass through the same
  // controller as "Identify speakers", so its progress shows on that button.
  const offerAudioSetup =
    !!meetingId &&
    !!onSetAudioSetup &&
    meetingOrigin !== 'imported' &&
    meetingOrigin !== 'notes_only' &&
    !speakersGoneTitle;
  const handleChooseAudioSetup = useCallback(
    (next: AudioSetupOverride) => {
      if (!onSetAudioSetup) return;
      void identifySpeakers(() => onSetAudioSetup(next));
    },
    [identifySpeakers, onSetAudioSetup],
  );

  return (
    <div className="flex items-center justify-start gap-2">
      <ButtonGroup>
        {meetingId && (
          <Button
            size="xs"
            variant="outline"
            className="gap-1.5"
            onClick={handleIdentifySpeakers}
            disabled={isDiarizing || transcriptCount === 0 || !!speakersGoneTitle}
            title={
              transcriptCount === 0
                ? 'No transcript to analyze'
                : speakersGoneTitle
                  ? speakersGoneTitle
                  : isDiarizing
                    ? diarizationStatus
                      ? `Identifying speakers: ${diarizationStatus}…`
                      : 'Identifying speakers…'
                    : identifyFailed
                      ? AUDIO_FAILED_NOTE
                      : 'Identify who spoke (runs on-device after the meeting)'
            }
          >
            {isDiarizing && <Loader2 className="animate-spin" size={16} />}
            <span>
              {isDiarizing
                ? (downloadProgress?.label ??
                  (hasRealProgress
                    ? `Identifying… ${diarizationPct}%`
                    : (diarizationStage ?? 'Identifying…')))
                : 'Identify speakers'}
            </span>
          </Button>
        )}

        {/* Deferred meeting (1.10 feedback): one button runs the full pipeline
            (transcribe → diarize → summarize) and clears the defer marker. */}
        {offerProcessNow && (
          <Button
            size="xs"
            variant="outline"
            className="bg-brand/10 hover:bg-brand/20 border-brand/30 text-brand gap-1.5"
            onClick={handleProcessNow}
            disabled={isProcessingThis}
            title={
              isProcessingThis
                ? 'Processing this meeting — see status in the indicator'
                : "This meeting hasn't been processed yet — transcribe, identify speakers, and summarize it now"
            }
          >
            {isProcessingThis && <Loader2 className="animate-spin" size={16} />}
            <span>
              {backlogStatus === 'waiting'
                ? 'Queued'
                : isProcessingThis
                  ? 'Processing…'
                  : 'Process now'}
            </span>
          </Button>
        )}

        {/* Deferred first-time transcription (specs/0029 WS7.2): the meeting has
            audio but no real transcript (recorded with live transcription off). */}
        {needsFirstTranscription && !isDeferred && (
          <Button
            size="xs"
            variant="outline"
            className="bg-brand/10 hover:bg-brand/20 border-brand/30 text-brand"
            onClick={() => {
              setShowRetranscribeDialog(true);
            }}
            title="This meeting was recorded without live transcription — transcribe its audio now"
          >
            <span>Transcribe now</span>
          </Button>
        )}

        {!needsFirstTranscription && meetingId && meetingFolderPath && (
          <Button
            size="xs"
            variant="outline"
            onClick={() => {
              setShowRetranscribeDialog(true);
            }}
            disabled={!!transcribeGoneTitle}
            title={transcribeGoneTitle ?? 'Retranscribe to enhance your recorded audio'}
          >
            <span>Enhance</span>
          </Button>
        )}

        {/* Copy and Open folder moved in here on owner feedback 2026-09-21: they were the
            first two buttons on the bar, so while a pass was running the row read
            "Copy · Open folder · Identifying… 42% · Enhance" — two file-management
            affordances sitting in front of the one thing actually happening. Same `…`
            pattern (and position) as SummaryToolbar, so the two document tabs now share
            one shape. */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="xs"
              aria-label="More transcript actions"
              title="More transcript actions"
            >
              <MoreHorizontal size={14} />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onSelect={() => onCopyTranscript()}
              disabled={transcriptCount === 0}
            >
              Copy
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void onOpenMeetingFolder()}>
              Open folder
            </DropdownMenuItem>
            {offerAudioSetup && (
              <>
                <DropdownMenuSeparator />
                <AudioSetupSubmenu
                  setup={audioSetup}
                  disabled={isDiarizing || transcriptCount === 0}
                  onChoose={handleChooseAudioSetup}
                />
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </ButtonGroup>

      {meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
