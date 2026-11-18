'use client';

import { useEffect, useRef, useState } from 'react';
import { PageHeader } from '@/components/ui/page-header';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { TITLE_PLACEHOLDER } from '@/hooks/useRecordingTitleEdit';
import { cn } from '@/lib/utils';
import { ParticipantsPopover } from '@/components/Participants/ParticipantsPopover';
import { VU_DEFAULT_HEIGHT, VuMeter } from '@/components/Transport/VuMeter';
import { useRecordingLevel } from '@/hooks/useRecordingLevel';
import { useRecordEmptyPhase } from '@/hooks/useRecordEmptyPhase';
import { rmsToVu } from '@/lib/transport/vu-ballistics';
import type { UseRecordingTitleEditReturn } from '@/hooks/useRecordingTitleEdit';
import { PromptPicker } from '@/components/SummaryPrompts/PromptPicker';
import { CustomPromptFlow, type PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';
import { useBack } from '@/hooks/useBack';
import type { RecordView } from '@/lib/record-view';
import { RecordViewTabs } from './RecordViewTabs';

/** px between the meters and the band's bottom rule (the right cluster's `pb-4`). */
const METER_BOTTOM_GAP = 16;

/** The faceplate band's bevel: a highlight above, a shade below, from the bevel tokens. */
const FACEPLATE_BEVEL = 'shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo))]'; // ui-gate: allow hardware faceplate bevel built from the --bevel-hi/--bevel-lo tokens; no card shadow fits a full-bleed band

interface RecordingHeaderProps {
  meetingTitle: string;
  isRecordingActive: boolean;
  activeRecordingMeetingId: string | null;
  titleEdit: UseRecordingTitleEditReturn;
  /** Per-meeting prompt picker's slice of useSummaryPrompts (specs/0079). */
  promptsApi: PromptsApi;
  /** Transcript / Focus (owner feedback, v0.18); the switch is hidden without a handler. */
  view?: RecordView;
  onViewChange?: (view: RecordView) => void;
}

/**
 * Recording header, specs/0057 Plan 2: a control-panel faceplate — back · title · the
 * Transcript / Focus tabs under it (v0.18) · participants · prompt · two channel VU meters. The transport rail (mounted in
 * the app layout) owns REC/HOLD/STOP, the reels and the tape counter, so nothing here
 * duplicates a control or a clock any more.
 */
export function RecordingHeader({
  meetingTitle,
  isRecordingActive,
  activeRecordingMeetingId,
  titleEdit,
  promptsApi,
  view = 'transcript',
  onViewChange,
}: RecordingHeaderProps) {
  const back = useBack('/');
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
  const hasViewTabs = !!onViewChange;
  useEffect(() => {
    const gap = hasViewTabs ? METER_BOTTOM_GAP : 0;
    // The ref sits on the subtitle node; PageHeader wraps it once (the `.u-meta` box), and
    // that box's parent is the title column (the h1 plus the subtitle) the meters follow.
    const el = titleColumnRef.current?.parentElement?.parentElement;
    if (!el || typeof ResizeObserver === 'undefined') return;
    // Less the band's bottom gap: the view tabs sit on the band's bottom rule, the meters
    // stop 16px above it (the right cluster's pb-4), so both columns end on the same line.
    const observer = new ResizeObserver(([entry]) => {
      setMeterHeight(Math.max(VU_DEFAULT_HEIGHT, Math.round(entry.contentRect.height) - gap));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasViewTabs]);

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
      {/* The view tabs sit under the title, like every other page's tabs (owner feedback,
          v0.18); the per-meeting controls that used to live here moved to the right-hand
          cluster, and Pause transcript into the transcript's own status strip. Their
          underline lands on the band's bottom rule, so they draw no rule of their own. */}
      {onViewChange && (
        <RecordViewTabs view={view} onViewChange={onViewChange} className="mt-2.5 border-b-0" />
      )}
    </div>
  );

  // Per-meeting controls, right of the title and left of the meters (owner feedback, v0.18):
  // who is here, then what the summary will look like.
  const controls =
    isRecordingActive || activeRecordingMeetingId ? (
      <>
        {/* Participants for the in-progress meeting (specs/0017). Use the authoritative
            SQLite meeting id (`activeRecordingMeetingId`, set at recording start), NOT the
            fabricated `currentMeetingId` from TranscriptContext (a `meeting-<timestamp>`
            IndexedDB id) — adds made against that orphan id are lost at stop (specs/0024
            WS3.1). The roster row + its calendar_event_id already exist from Join & Record,
            so seeding works live; an ad-hoc recording starts empty and is hand-added. */}
        {activeRecordingMeetingId && <ParticipantsPopover meetingId={activeRecordingMeetingId} />}
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
              title="Summary prompt — used when this Meeting is summarized"
              className="h-8 max-w-[180px] text-muted-foreground"
            />
            <CustomPromptFlow
              open={customFlowOpen}
              onClose={() => setCustomFlowOpen(false)}
              promptsApi={promptsApi}
            />
          </>
        )}
      </>
    ) : null;

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
      // pb-0: the view tabs' underline sits on the band's bottom rule; the right cluster
      // keeps its own 16px (pb-4) below the meters.
      className={cn(
        'record-header border-b border-border bg-panel pr-4 pt-4 min-[900px]:pr-4',
        hasViewTabs ? 'pb-0' : 'pb-4',
        FACEPLATE_BEVEL,
      )}
      back={{ label: 'Back', onClick: back }}
      title={
        <MeetingTitleField
          title={realTitle}
          placeholder={isRecordingActive ? 'Untitled Meeting' : 'New Meeting'}
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
      actions={
        <div className={cn('flex items-center gap-2.5', hasViewTabs && 'pb-4')}>
          {controls}
          {meters}
        </div>
      }
    />
  );
}
