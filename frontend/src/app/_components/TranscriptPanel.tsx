import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { PermissionWarning } from '@/components/PermissionWarning';
import { AlertTriangle, BatteryLow, MicOff } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { expandTranscriptSegments } from '@/lib/live-channel-split';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useIsLinux } from '@/hooks/usePlatform';
import { useProcessingMode } from '@/hooks/useProcessingMode';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { effectiveLiveTranscription, transcriptEmptyStateVariant } from '@/lib/processing-mode';
import { useEffect, useMemo, useState } from 'react';

/**
 * TranscriptPanel Component
 *
 * Displays live transcript content and recording-status banners.
 * Uses TranscriptContext and RecordingStateContext internally.
 */

interface TranscriptPanelProps {
  // indicates stop-processing state for transcripts; derived from backend statuses.
  isProcessingStop: boolean;
  isStopping: boolean;
}

export function TranscriptPanel({
  isProcessingStop,
  isStopping,
}: TranscriptPanelProps) {
  // Contexts
  const { transcripts, transcriptGaps } = useTranscripts();
  const { isRecording, isPaused } = useRecordingState();
  const { checkPermissions, isChecking, hasSystemAudio, hasMicrophone } = usePermissionCheck();
  const isLinux = useIsLinux();
  const { activeRecordingMeetingId } = useSidebar();

  // Live transcription state (specs/0029 WS7.2; low-power-mode spec §§3,5). When OFF
  // (record-only mode) the backend never emits transcript updates, so instead of a
  // dead empty transcript we show a quiet explanatory state. Prefer the live value
  // from useProcessingMode()'s `processing-mode-changed` event — it reflects
  // mid-meeting overrides (e.g. the header chip, or plugging in) — and fall back to
  // the stored preference until the first event of this session arrives.
  const { liveTranscription: liveTranscriptionEvent, onBattery } = useProcessingMode();
  const [liveTranscriptionPref, setLiveTranscriptionPref] = useState(true);
  useEffect(() => {
    let cancelled = false;
    invoke<{ live_transcription_enabled?: boolean }>('get_recording_preferences')
      .then((prefs) => {
        if (!cancelled) {
          setLiveTranscriptionPref(prefs?.live_transcription_enabled !== false);
        }
      })
      .catch((error) => {
        console.error('Failed to load recording preferences for transcript panel:', error);
      });
    return () => {
      cancelled = true;
    };
  }, [isRecording]);
  const liveTranscriptionEnabled = effectiveLiveTranscription(liveTranscriptionEvent, liveTranscriptionPref);

  // Per-meeting "go live" override from the low-power empty state (low-power-mode
  // spec §5). On error, toast and revert nothing — the real state comes back via the
  // next `processing-mode-changed` event, same as the header chip.
  const [enablingLive, setEnablingLive] = useState(false);
  const handleTranscribeLiveNow = async () => {
    if (!activeRecordingMeetingId || enablingLive) return;
    setEnablingLive(true);
    try {
      await invoke('api_set_meeting_processing_mode', { meetingId: activeRecordingMeetingId, mode: 'live' });
      await invoke('api_apply_live_transcription_now', { enable: true });
    } catch (error) {
      console.error('Failed to switch this meeting to live transcription:', error);
      toast.error('Failed to turn on live transcription for this meeting');
    } finally {
      setEnablingLive(false);
    }
  };

  // Convert transcripts to segments for virtualized view.
  //
  // Live speaker diarization (specs/0011, P3-B): `speaker` is the color key,
  // `speakerName` the shown label. Fed from `transcript-update.speaker` and
  // patched in place by `live-diarization-update` (see TranscriptContext).
  // Absent until labeled — a segment with no label shows no speaker chip.
  //
  // specs/0055: a row whose `channel_runs` cross an owner<->remote handoff is
  // expanded into one segment per side, so the owner's opening words stop
  // carrying the previous speaker's name (and vice versa). Render-time only —
  // the rows in context state, and therefore the save payload, are unchanged.
  const segments = useMemo(
    () => expandTranscriptSegments(transcripts),
    [transcripts]
  );

  // Show the "labels are provisional" hint only once a live label has appeared
  // and only while recording (the post-meeting view shows the authoritative ones).
  const hasLiveSpeakerLabel = useMemo(
    () => isRecording && transcripts.some(t => t.speaker_name),
    [isRecording, transcripts]
  );

  // Backpressure gap hint (specs/0028, surfaced in 0030 WS4): when the transcription
  // pipeline sheds audio — queue saturation (`dropped`) or an unavailable speech model
  // (`skipped`) — the transcript has gaps. Unlike the coalesced toasts (which expire),
  // this banner persists for the rest of the session so the user knows the live
  // transcript may be missing audio. Counts reset when the next recording starts.
  const gapMessage = useMemo(() => {
    const { dropped, skipped } = transcriptGaps;
    const total = dropped + skipped;
    if (total === 0) return null;
    const seg = (n: number) => `${n} audio segment${n === 1 ? '' : 's'}`;
    if (dropped > 0 && skipped > 0) {
      return `${seg(total)} may be missing from this transcript (${dropped} skipped to catch up, ${skipped} not transcribed).`;
    }
    if (dropped > 0) {
      return `Transcription ${isRecording ? 'is behind' : 'fell behind'} — ${seg(dropped)} ${dropped === 1 ? 'was' : 'were'} skipped to catch up and may be missing from this transcript.`;
    }
    return `${seg(skipped)} could not be transcribed (speech model unavailable) and may be missing from this transcript.`;
  }, [transcriptGaps, isRecording]);

  // Which record-only empty-state to show, if any (low-power-mode spec §5): null once
  // live transcription is on or a transcript has content.
  const emptyStateVariant = useMemo(
    () => transcriptEmptyStateVariant(liveTranscriptionEnabled, transcripts.length > 0, onBattery),
    [liveTranscriptionEnabled, transcripts.length, onBattery],
  );

  // specs/0029 WS4.1 — exactly ONE element scrolls in this panel: the scroll container
  // inside VirtualizedTranscriptView, owned by useAutoScroll's hysteresis follower.
  // This outer div is a non-scrolling flex column (header fixed, transcript area bounded
  // via flex-1/min-h-0), matching the meeting-details layout pattern.
  return (
    <div className="w-full min-h-0 flex-1 border-r border-border bg-paper flex flex-col overflow-hidden">
      {/* Fixed header — only rendered when there's a status banner to show. The
          transcription-language picker moved to a single global control in
          Settings (spec 0038 WS7.d); the old in-recording Language modal is gone. */}
      {(hasLiveSpeakerLabel || gapMessage) && (
        <div className="bg-card px-5 py-3 border-border">
          <div className="flex flex-col space-y-2">
            {/* Provisional-labels affordance (specs/0011, P3-B): live speaker
                labels are best-effort and reconciled to the authoritative offline
                pass when recording stops. Subtle, non-nagging, only while live
                labels are actually showing. */}
            {hasLiveSpeakerLabel && (
              <p className="u-section-label text-brand">
                Provisional — speaker labels are finalized when you stop recording.
              </p>
            )}
            {/* Persistent backpressure hint (specs/0028 → 0030 WS4): the transcript has
                gaps — one banner with a running count instead of a toast per event. */}
            {gapMessage && (
              <div
                role="status"
                className="flex items-start gap-2 rounded-md border border-brand/30 bg-brand/10 px-3 py-2"
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-brand" aria-hidden="true" />
                <p className="text-xs text-foreground">{gapMessage}</p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Permission Warning - Not needed on Linux */}
      {!isRecording && !isChecking && !isLinux && (
        <div className="flex justify-center px-4 pt-4">
          <PermissionWarning
            hasMicrophone={hasMicrophone}
            hasSystemAudio={hasSystemAudio}
            onRecheck={checkPermissions}
            isRechecking={isChecking}
          />
        </div>
      )}

      {/* Transcript content — full width of the panel (no centered narrow column).
          flex-1/min-h-0 bounds the height so the VirtualizedTranscriptView root
          actually overflows and becomes the single scroll owner (0029 WS4.1). */}
      {emptyStateVariant === 'low-power-battery' ? (
        /* Low Power Mode, deferred because we're on battery (low-power-mode spec §5):
           offer a one-click per-meeting override instead of just explaining. */
        <div className="flex-1 min-h-0 px-5 pb-4 flex items-center justify-center">
          <div className="text-center max-w-xs space-y-3">
            <BatteryLow className="mx-auto h-5 w-5 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm font-medium text-muted-foreground">
              Low Power Mode — recording only
            </p>
            <p className="text-xs text-muted-foreground">
              Transcription resumes when you&apos;re plugged in, or turn it on for this
              meeting.
            </p>
            {activeRecordingMeetingId && (
              <button
                type="button"
                onClick={handleTranscribeLiveNow}
                disabled={enablingLive}
                className="inline-flex items-center gap-1.5 rounded-md border border-input bg-card px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
              >
                {enablingLive ? 'Starting…' : 'Transcribe this meeting live'}
              </button>
            )}
          </div>
        </div>
      ) : emptyStateVariant === 'live-transcription-off' ? (
        /* Record-only mode (specs/0029 WS7.2): no live STT runs, so an empty
           transcript is expected — say so quietly instead of looking broken.
           This variant also covers a session deferred on battery that has since
           been plugged into AC mid-recording (low-power-mode Finding 4): the
           battery-specific copy would then mislead, so while recording we show
           neutral deferred copy AND still offer the per-meeting go-live button. */
        <div className="flex-1 min-h-0 px-5 pb-4 flex items-center justify-center">
          <div className="text-center max-w-xs space-y-3">
            <MicOff className="mx-auto h-5 w-5 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm font-medium text-muted-foreground">
              Live transcription is off
            </p>
            <p className="text-xs text-muted-foreground">
              {isRecording
                ? 'Transcription is deferred for this meeting — turn it on below or it will be processed after the meeting.'
                : 'Recordings are saved without a live transcript and transcribed later. You can turn live transcription back on in Settings → Recording.'}
            </p>
            {isRecording && activeRecordingMeetingId && (
              <button
                type="button"
                onClick={handleTranscribeLiveNow}
                disabled={enablingLive}
                className="inline-flex items-center gap-1.5 rounded-md border border-input bg-card px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
              >
                {enablingLive ? 'Starting…' : 'Transcribe this meeting live'}
              </button>
            )}
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 px-5 pb-4">
          <VirtualizedTranscriptView
            segments={segments}
            isRecording={isRecording}
            isPaused={isPaused}
            isProcessing={isProcessingStop}
            isStopping={isStopping}
            enableStreaming={isRecording}
            showConfidence={true}
            // specs/0071 W2 — so the in-list indicator can say "Transcript paused" rather
            // than "Listening…" while VAD/STT are detached. Already resolved above with the
            // stored-preference fallback, so it is correct before the first event lands.
            liveTranscription={liveTranscriptionEnabled}
          />
        </div>
      )}
    </div>
  );
}
