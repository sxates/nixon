'use client';

import { useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { BatteryLow, Check, ChevronDown, ChevronLeft, Pencil } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
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
import type { useTemplates } from '@/hooks/meeting-details/useTemplates';

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
    <button
      type="button"
      onClick={handleClick}
      disabled={enabling}
      // The action, matching the visible label — a screen reader must hear the same thing an
      // eye reads (specs/0071 W1). It used to announce the mode and then the switch, which is
      // the same trap in longer form.
      aria-label={label}
      // h-8 matches the template picker and the participants trigger beside it — the three
      // per-meeting controls used to be h-7 / h-9 / h-9 and visibly failed to line up
      // (owner feedback 2026-09-21).
      className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border bg-card px-3 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
    >
      {showBatteryGlyph && <BatteryLow size={14} />}
      {enabling ? 'Switching…' : label}
    </button>
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

/** The per-meeting template picker's slice of useTemplates (specs/0029 WS4.3). */
type TemplatesApi = Pick<
  ReturnType<typeof useTemplates>,
  'availableTemplates' | 'selectedTemplate' | 'handleTemplateSelection'
>;

interface RecordingHeaderProps {
  meetingTitle: string;
  isRecordingActive: boolean;
  activeRecordingMeetingId: string | null;
  titleEdit: UseRecordingTitleEditReturn;
  templates: TemplatesApi;
}

/**
 * Recording header, specs/0057 Plan 2: a control-panel faceplate — back · title · identity
 * line · template · mode · participants · two channel VU meters + lamps. The transport rail (mounted in
 * the app layout) owns REC/HOLD/STOP, the reels and the tape counter, so nothing here
 * duplicates a control or a clock any more.
 */
export function RecordingHeader({
  meetingTitle,
  isRecordingActive,
  activeRecordingMeetingId,
  titleEdit,
  templates,
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
  const { availableTemplates, selectedTemplate, handleTemplateSelection } = templates;
  const selectedTemplateName =
    availableTemplates.find((t) => t.id === selectedTemplate)?.name ?? 'Template';
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
    const el = titleColumnRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => {
      setMeterHeight(Math.max(VU_DEFAULT_HEIGHT, Math.round(entry.contentRect.height)));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <header className="record-header flex items-start gap-x-2 border-b border-border bg-panel py-4 pl-4 pr-4 min-[900px]:pl-6 shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo))]">
      {/* 0.1.0 canvas feedback: the back control is the same unboxed chevron as on meeting
          details, sitting on the title line (the header top-aligns for that; the meter
          bridge re-centres itself on the right). */}
      <button
        onClick={() => router.push('/')}
        aria-label="Back to home"
        title="Back to home"
        className="-ml-1 mt-0.5 inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronLeft size={18} />
      </button>

      <div ref={titleColumnRef} className="min-w-0 flex-1 basis-60">
        {/* Editable during a recording (specs/0029 WS4.3); static until the recording's
            SQLite row id exists — there is nothing to rename before that. */}
        {!isRecordingActive || !activeRecordingMeetingId ? (
          <h1 className="font-display truncate text-lg font-semibold">
            {meetingTitle}
          </h1>
        ) : isEditingTitle ? (
          <input
            ref={titleInputRef}
            type="text"
            value={titleDraft}
            placeholder="Untitled meeting"
            onChange={(e) => setTitleDraft(e.target.value)}
            onBlur={commitTitleEdit}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                commitTitleEdit();
              } else if (e.key === 'Escape') {
                e.preventDefault();
                cancelTitleEdit();
              }
            }}
            className="w-full max-w-md rounded-md border border-input bg-muted px-2 py-0.5 font-display text-lg font-semibold text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          />
        ) : (
          <button
            type="button"
            onClick={startEditingTitle}
            title="Click to rename"
            className="group -ml-2 flex w-fit max-w-full items-center gap-2 rounded-md px-2 py-0.5 text-left transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <h1 className="font-display truncate text-lg font-semibold">
              {meetingTitle}
            </h1>
            <Pencil className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
          </button>
        )}
        {/* No "On the reel" subhead while recording (owner feedback 2026-09-21): the
            transport rail says exactly that, in the same words, a few inches below — and
            the only way to be looking at this header at all is with a recording in
            progress, so the line carried no information for the height it took. The idle
            line stays: an idle header DOES need to say what pressing REC will do. */}
        {!isRecordingActive && !transition && (
          <p className="mt-0.5 text-xs text-muted-foreground">Recording locally on your Mac</p>
        )}
        {/* 0.1.0 canvas feedback: the per-meeting controls sit under the title, not in the
            meter bridge. Rendered only when at least one is live so an idle header carries
            no empty row. */}
        {/* Per-meeting controls, ordered Participants → Live → Template (owner feedback
            2026-09-21): who is here, then how it is being processed, then what the summary
            will look like — decreasing immediacy, left to right. All three are h-8; they
            used to be h-9 / h-9 / h-7 and the row visibly stepped. Rendered only when at
            least one is live, so an idle header carries no empty row. */}
        {(isRecordingActive && availableTemplates.length > 0) || activeRecordingMeetingId ? (
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
            {/* Per-meeting summary template picker (specs/0029 WS4.3): quiet dropdown,
                persisted against the recording's SQLite id; the eventual summary uses it. */}
            {isRecordingActive && availableTemplates.length > 0 && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="outline"
                    size="xs"
                    title="Summary template — used when this meeting is summarized"
                    className="h-8 max-w-[180px] text-muted-foreground"
                  >
                    <span className="truncate">{selectedTemplateName}</span>
                    <ChevronDown className="h-3 w-3 flex-shrink-0" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  {availableTemplates.map((template) => (
                    <DropdownMenuItem
                      key={template.id}
                      onClick={() => handleTemplateSelection(template.id, template.name)}
                      title={template.description}
                      className="flex items-center justify-between gap-2"
                    >
                      <span>{template.name}</span>
                      {selectedTemplate === template.id && (
                        <Check className="h-4 w-4 text-brand" />
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
        ) : null}
      </div>

      {/* The control panel's meter bridge (specs/0057 §3.2): one needle per channel — CH1
          is the owner's mic, CH2 is system audio (the other side of the call) — read from
          the clean pre-mix windows. The PEAK / MIC GATE lamps were dropped on 0.1.0 canvas
          feedback: the rail's ladder and the transport status line carry both states. The
          meters carry their own channel label inside the well since 2026-09-21 — see
          VuMeter. Since 2026-09-23 they are as tall as the title column (57px minimum)
          rather than floating centred beside it, and the header's right padding is 16px to
          match the 16px above and below them (owner feedback).
          Hidden when the header's content is under 804px wide (owner feedback 2026-09-23) — a
          container query on the header, not the window, so an open or closed sidebar is
          accounted for (globals.css `.record-meters`). The header no longer wraps: wrapped
          under the controls, the meters' size fed back into the title column's height they
          are sized from, and they flickered between the two positions. */}
      <div className="record-meters flex-shrink-0 items-start gap-2.5">
        <VuMeter
          db={rmsToVu(level.mic.rms)}
          active={isRecordingActive}
          label="CH1 Mic"
          height={meterHeight}
        />
        <VuMeter
          db={rmsToVu(level.sys.rms)}
          active={isRecordingActive}
          label="CH2 Sys"
          height={meterHeight}
        />
      </div>
    </header>
  );
}
