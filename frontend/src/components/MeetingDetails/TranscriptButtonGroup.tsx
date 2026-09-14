"use client";

import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Loader2 } from 'lucide-react';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useDiarization } from '@/hooks/useDiarization';
import { SPARSE_TRANSCRIPT_SEGMENTS } from '@/lib/deferred-transcription';
import { isMeetingInFlight } from '@/lib/deferred-backlog';

/** Tooltip when the meeting's audio files are gone (specs/0029 WS7.1) — most
 * commonly removed by the "Delete recording audio after N days" setting. */
const AUDIO_UNAVAILABLE_TITLE =
  'No audio recording available — it may have been removed by your audio retention setting';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
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

  // Does this meeting still have audio on disk? (specs/0029 WS7.1). The retention
  // sweep deletes only media files, so a meeting can have a transcript but no audio —
  // diarize/re-transcribe would fail-fast with a raw error. Probe once per meeting and
  // gate those affordances with a friendly state instead. null = unknown (probe pending
  // or failed) — don't gate on unknown, the flows' own errors still backstop.
  const [audioAvailable, setAudioAvailable] = useState<boolean | null>(null);
  useEffect(() => {
    setAudioAvailable(null);
    if (!meetingId) return;
    let cancelled = false;
    invoke<boolean>('api_meeting_audio_available', { meetingId })
      .then((available) => {
        if (!cancelled) setAudioAvailable(available);
      })
      .catch((error) => {
        console.error('Failed to check audio availability:', error);
        if (!cancelled) setAudioAvailable(null);
      });
    return () => {
      cancelled = true;
    };
  }, [meetingId]);
  const audioRemoved = audioAvailable === false;

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

  return (
    <div className="flex items-center justify-start gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="xs"
          onClick={() => {
            onCopyTranscript();
          }}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? 'No transcript available' : 'Copy Transcript'}
        >
          <span>Copy</span>
        </Button>

        <Button
          size="xs"
          variant="outline"
          onClick={() => {
            onOpenMeetingFolder();
          }}
          title="Open Recording Folder"
        >
          <span>Open folder</span>
        </Button>

        {meetingId && (
          <Button
            size="xs"
            variant="outline"
            className="gap-1.5"
            onClick={handleIdentifySpeakers}
            disabled={isDiarizing || transcriptCount === 0 || audioRemoved}
            title={
              transcriptCount === 0
                ? 'No transcript to analyze'
                : audioRemoved
                  ? AUDIO_UNAVAILABLE_TITLE
                  : isDiarizing
                    ? diarizationStatus
                      ? `Identifying speakers: ${diarizationStatus}…`
                      : 'Identifying speakers…'
                    : 'Identify who spoke (runs on-device after the meeting)'
            }
          >
            {isDiarizing && <Loader2 className="animate-spin" size={16} />}
            <span>
              {isDiarizing
                ? hasRealProgress
                  ? `Identifying… ${diarizationPct}%`
                  : 'Identifying…'
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

        {betaFeatures.importAndRetranscribe && !needsFirstTranscription && meetingId && meetingFolderPath && (
          <Button
            size="xs"
            variant="outline"
            className="bg-brand/10 hover:bg-brand/20 border-brand/30 text-brand"
            onClick={() => {
              setShowRetranscribeDialog(true);
            }}
            disabled={audioRemoved}
            title={
              audioRemoved
                ? AUDIO_UNAVAILABLE_TITLE
                : 'Retranscribe to enhance your recorded audio'
            }
          >
            <span>Enhance</span>
          </Button>
        )}
      </ButtonGroup>

      {(betaFeatures.importAndRetranscribe || needsFirstTranscription) &&
        meetingId &&
        meetingFolderPath && (
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
