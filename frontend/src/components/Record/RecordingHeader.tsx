'use client';

import { useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { BatteryLow } from '@/components/ui/icons';
import { PageHeader } from '@/components/ui/page-header';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { TITLE_PLACEHOLDER } from '@/hooks/useRecordingTitleEdit';
import { cn } from '@/lib/utils';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { ParticipantsPopover } from '@/components/Participants/ParticipantsPopover';
import { VU_DEFAULT_HEIGHT, VuMeter } from '@/components/Transport/VuMeter';
import { useProcessingMode } from '@/hooks/useProcessingMode';
import { useRecordingLevel } from '@/hooks/useRecordingLevel';
import { useRecordEmptyPhase } from '@/hooks/useRecordEmptyPhase';
import { modeChipDisplay } from '@/lib/processing-mode';
import { rmsToVu } from '@/lib/transport/vu-ballistics';
import type { UseRecordingTitleEditReturn } from '@/hooks/useRecordingTitleEdit';
import { PromptPicker } from '@/components/SummaryPrompts/PromptPicker';
import { CustomPromptFlow, type PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';

/**
 * Compact per-meeting live/defer mode chip (low-power-mode spec §5). Driven by
 * `useProcessingMode()`'s live `processing-mode-changed` event; clicking flips the
 * mode for the current recording via `api_set_meeting_processing_mode` +
 * `api_apply_live_transcription_now`. Renders nothing until the first event of this
 * session has arrived, so it never flashes a guessed state.
 */
function ModeChip({ meetingId }: { meetingId: string }) {
  const { liveTranscription, onBattery } = useProcessingMode();
  // Only the live-going transition waits on the STT model spinning up; deferring is
  // effectively instant. Track it separately so the tooltip copy stays accurate.
  const [enabling, setEnabling] = useState(false);

  if (liveTranscription === null) return null;
  const { label, showBatteryGlyph } = modeChipDisplay(liveTranscription, onBattery);

  const handleClick = async () => {
    if (enabling) return;
    const goingLive = !liveTranscription;
    if (goingLive) setEnabling(true);
    try {
      await invoke('api_set_meeting_processing_mode', {
        meetingId,
        mode: goingLive ? 'live' : 'defer',
      });
      await invoke('api_apply_live_transcription_now', { enable: goingLive });
    } catch (error) {
      console.error('Failed to change this meeting\'s transcription mode:', error);
      toast.error('Failed to change transcription mode');
    } finally {
      if (goingLive) setEnabling(false);
    }
  };

  const chip = (
    <Button
      type="button"
      variant="outline"
      // sm = h-8: matches the prompt picker and the participants trigger beside it — the
      // three per-meeting controls used to be h-7 / h-9 / h-9 and visibly failed to line up
      // (owner feedback 2026-09-21).
      size="sm"
      onClick={handleClick}
      disabled={enabling}
      // The action, matching the visible label — a screen reader must hear the same thing an
      // eye reads (specs/0071 W1). It used to announce the mode and then the switch, which is
      // the same trap in longer form.
      aria-label={label}
      className="bg-card"
    >
      {/* The only carrier of WHY the transcript is paused (on battery): functional, not
          decorative, so it stays. */}
      {showBatteryGlyph && <DeckIcon icon={BatteryLow} size={14} />}
      {enabling ? 'Switching…' : label}
    </Button>
  );

  if (!enabling) return chip;

  return (
    <TooltipProvider>
      <Tooltip open>
        <TooltipTrigger asChild>{chip}</TooltipTrigger>
        <TooltipContent>waiting for model…</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** The faceplate band's bevel: a highlight above, a shade below, from the bevel tokens. */
const FACEPLATE_BEVEL = 'shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo))]'; // ui-gate: allow hardware faceplate bevel built from the --bevel-hi/--bevel-lo tokens; no card shadow fits a full-bleed band

interface RecordingHeaderProps {
  meetingTitle: string;
  isRecordingActive: boolean;
  activeRecordingMeetingId: string | null;
  titleEdit: UseRecordingTitleEditReturn;
  /** Per-meeting prompt picker's slice of useSummaryPrompts (specs/0079). */
  promptsApi: PromptsApi;
}

/**
 * Recording header, specs/0057 Plan 2: a control-panel faceplate — back · title · identity
 * line · prompt · mode · participants · two channel VU meters + lamps. The transport rail (mounted in
 * the app layout) owns REC/HOLD/STOP, the reels and the tape counter, so nothing here
 * duplicates a control or a clock any more.
 */
export function RecordingHeader({
  meetingTitle,
  isRecordingActive,
  activeRecordingMeetingId,
  titleEdit,
  promptsApi,
}: RecordingHeaderProps) {
  const router = useRouter();
  const {
    isEditingTitle,
    titleDraft,
    setTitleDraft,
    titleInputRef,
    startEditingTitle,
    commitTitleEdit,
    cancelTitleEdit,
  } = titleEdit;
  const [customFlowOpen, setCustomFlowOpen] = useState(false);
  const level = useRecordingLevel(isRecordingActive);
  // Starting or saving is not idle: the idle subhead would flash on the way in and out.
  const transition = useRecordEmptyPhase();

  // The meters take the title column's height (never under their 57px default), so they
  // fill the header instead of floating in it without making it any taller (owner
  // feedback 2026-09-23). Measured, because a CSS stretch lets the meters' own
  // aspect-derived height feed back into the row they are sizing themselves from.
  const titleColumnRef = useRef<HTMLDivElement>(null);
  const [meterHeight, setMeterHeight] = useState(VU_DEFAULT_HEIGHT);
  useEffect(() => {
    // The ref sits on the subtitle node; PageHeader wraps it once (the `.u-meta` box), and
    // that box's parent is the title column (the h1 plus the subtitle) the meters follow.
    const el = titleColumnRef.current?.parentElement?.parentElement;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => {
      setMeterHeight(Math.max(VU_DEFAULT_HEIGHT, Math.round(entry.contentRect.height)));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // The context's unnamed-session placeholder ('+ New Call') is not a title: an idle
  // recorder reads "New meeting", an unnamed live one "Untitled meeting", both muted.
  const realTitle = meetingTitle === TITLE_PLACEHOLDER ? '' : meetingTitle;
  // Editable during a recording (specs/0029 WS4.3); static until the recording's SQLite
  // row id exists — there is nothing to rename before that.
  const canRename = isRecordingActive && !!activeRecordingMeetingId;

  const subtitle = (
    // Measured: the meters take the title column's height (see above).
    <div ref={titleColumnRef}>
      {/* No "On the reel" subhead while recording (owner feedback 2026-09-21): the
          transport rail says exactly that, in the same words, a few inches below — and
          the only way to be looking at this header at all is with a recording in
          progress, so the line carried no information for the height it took. The idle
          line stays: an idle header DOES need to say what pressing REC will do. */}
      {!isRecordingActive && !transition && <p>Recording locally on your Mac</p>}
        {/* 0.1.0 canvas feedback: the per-meeting controls sit under the title, not in the
            meter bridge. Ordered Participants → Live → Prompt (owner feedback 2026-09-21):
            who is here, then how it is being processed, then what the summary will look
            like — decreasing immediacy, left to right. All three are h-8; they used to be
            h-9 / h-9 / h-7 and the row visibly stepped. Rendered only when at least one is
            live, so an idle header carries no empty row. */}
        {isRecordingActive || activeRecordingMeetingId ? (
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            {/* Participants for the in-progress meeting (specs/0017). Use the authoritative
                SQLite meeting id (`activeRecordingMeetingId`, set at recording start), NOT the
                fabricated `currentMeetingId` from TranscriptContext (a `meeting-<timestamp>`
                IndexedDB id) — adds made against that orphan id are lost at stop (specs/0024
                WS3.1). The roster row + its calendar_event_id already exist from Join & Record,
                so seeding works live; an ad-hoc recording starts empty and is hand-added. */}
            {activeRecordingMeetingId && (
              <ParticipantsPopover meetingId={activeRecordingMeetingId} />
            )}
            {/* Per-meeting live/defer mode chip (low-power-mode spec §5) — only while an
                actual recording session is active, same gate as ParticipantsPopover. */}
            {isRecordingActive && activeRecordingMeetingId && (
              <ModeChip meetingId={activeRecordingMeetingId} />
            )}
            {/* Per-meeting summary prompt picker (specs/0079): persisted against the
                recording's SQLite id; the eventual summary uses it. */}
            {isRecordingActive && (
              <>
                <PromptPicker
                  prompts={promptsApi.prompts}
                  promptState={promptsApi.state}
                  onPromptSelect={(id) => {
                    void promptsApi.selectPrompt(id);
                  }}
                  onCustomPrompt={() => setCustomFlowOpen(true)}
                  onClearCustomPrompt={() => {
                    void promptsApi.clearOneOff();
                  }}
                  title="Summary prompt — used when this meeting is summarized"
                  className="h-8 max-w-[180px] text-muted-foreground"
                />
                <CustomPromptFlow
                  open={customFlowOpen}
                  onClose={() => setCustomFlowOpen(false)}
                  promptsApi={promptsApi}
                />
              </>
            )}
          </div>
        ) : null}
    </div>
  );

  // The control panel's meter bridge (specs/0057 §3.2): one needle per channel — CH1 is the
  // owner's mic, CH2 is system audio (the other side of the call) — read from the clean
  // pre-mix windows. The PEAK / MIC GATE lamps were dropped on 0.1.0 canvas feedback: the
  // rail's ladder and the transport status line carry both states. The meters carry their
  // own channel label inside the well since 2026-09-21 — see VuMeter. Since 2026-09-23 they
  // are as tall as the title column (57px minimum) rather than floating centred beside it
  // (owner feedback). Hidden when the header's content is under 804px wide (owner feedback
  // 2026-09-23) — a container query on the header, not the window, so an open or closed
  // sidebar is accounted for (globals.css `.record-header` / `.record-meters`). The header
  // never wraps: wrapped under the controls, the meters' size fed back into the title
  // column's height they are sized from, and they flickered between the two positions.
  const meters = (
    <div className="record-meters flex-shrink-0 items-start gap-2.5">
      <VuMeter db={rmsToVu(level.mic.rms)} active={isRecordingActive} label="CH1 Mic" height={meterHeight} />
      <VuMeter db={rmsToVu(level.sys.rms)} active={isRecordingActive} label="CH2 Sys" height={meterHeight} />
    </div>
  );

  // The same PageHeader + MeetingTitleField as the meeting page (one back/title header),
  // on the recorder's faceplate band: bg-panel, a hairline below, the panel bevel.
  return (
    <PageHeader
      // pr-4: the meters sit 16px from the right edge, matching the 16px above and below
      // them (owner feedback 2026-09-23).
      className={cn('record-header border-b border-border bg-panel pb-4 pr-4 pt-4 min-[900px]:pr-4', FACEPLATE_BEVEL)}
      back={{ label: 'Back to home', onClick: () => router.push('/') }}
      title={
        <MeetingTitleField
          title={realTitle}
          placeholder={isRecordingActive ? 'Untitled meeting' : 'New meeting'}
          editable={canRename}
          isEditing={isEditingTitle}
          draft={titleDraft}
          onDraftChange={setTitleDraft}
          onStartEditing={startEditingTitle}
          onCommit={commitTitleEdit}
          onCancel={cancelTitleEdit}
          inputRef={titleInputRef}
        />
      }
      subtitle={subtitle}
      actions={meters}
    />
  );
}
