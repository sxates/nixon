'use client';

import { useEffect, useRef, useState } from 'react';
import { PageHeader } from '@/components/ui/page-header';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { TITLE_PLACEHOLDER } from '@/hooks/useRecordingTitleEdit';
import { cn } from '@/lib/utils';
import { VU_DEFAULT_HEIGHT, VuMeter } from '@/components/Transport/VuMeter';
import { useRecordingLevel } from '@/hooks/useRecordingLevel';
import { useRecordEmptyPhase } from '@/hooks/useRecordEmptyPhase';
import { rmsToVu } from '@/lib/transport/vu-ballistics';
import type { UseRecordingTitleEditReturn } from '@/hooks/useRecordingTitleEdit';
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
  /** Transcript · My Notes · Agenda (owner feedback, v0.18); hidden without a handler. */
  view?: RecordView;
  onViewChange?: (view: RecordView) => void;
}

/**
 * Recording header, specs/0057 Plan 2: a control-panel faceplate. Left column: back · title,
 * then the Transcript · My Notes · Agenda tabs on the header's left edge (owner feedback, v0.18); the
 * two channel VU meters on the right. Participants and the summary prompt live in the
 * panes' top strips (RecordMeetingControls), Pause transcript in the transcript's. The transport rail (mounted in the app
 * layout) owns REC/HOLD/STOP, the reels and the tape counter, so nothing here duplicates a
 * control or a clock any more.
 */
export function RecordingHeader({
  meetingTitle,
  isRecordingActive,
  activeRecordingMeetingId,
  titleEdit,
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
  const level = useRecordingLevel(isRecordingActive);
  // Starting or saving is not idle: the idle subhead would flash on the way in and out.
  const transition = useRecordEmptyPhase();

  // The meters take the left column's height (never under their 57px default), so they
  // fill the header instead of floating in it without making it any taller (owner
  // feedback 2026-09-23). Measured, because a CSS stretch lets the meters' own
  // aspect-derived height feed back into the row they are sizing themselves from. The
  // measured box is the left column's content (title row + tabs row), which the band's
  // stretch never touches.
  const leftColumnRef = useRef<HTMLDivElement>(null);
  const [meterHeight, setMeterHeight] = useState(VU_DEFAULT_HEIGHT);
  const hasViewTabs = !!onViewChange;
  useEffect(() => {
    // With the tabs row, the tabs sit on the band's bottom rule and the meters stop 16px
    // above it (their own pb-4), so both columns end on the same line.
    const gap = hasViewTabs ? METER_BOTTOM_GAP : 0;
    const el = leftColumnRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
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

  // No "On the reel" subhead while recording (owner feedback 2026-09-21): the transport
  // rail says exactly that. The idle line stays: an idle header DOES need to say what
  // pressing REC will do.
  const subtitle = !isRecordingActive && !transition ? 'Recording locally on your Mac' : undefined;

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
    <div className="record-meters flex-shrink-0 items-start gap-2.5 py-4 pr-4">
      <VuMeter db={rmsToVu(level.mic.rms)} active={isRecordingActive} label="CH1 Mic" height={meterHeight} />
      <VuMeter db={rmsToVu(level.sys.rms)} active={isRecordingActive} label="CH2 Sys" height={meterHeight} />
    </div>
  );

  // The same PageHeader + MeetingTitleField as the meeting page (one back/title header),
  // on the recorder's faceplate band: bg-panel, a hairline below, the panel bevel. The band
  // owns the padding: 16px all round (the owner wanted less room left of the back key than
  // the page gutter's 28px), so the back glyph and the first tab label share the x = 16px
  // edge. The meters sit outside PageHeader so they can span both rows of the left column.
  return (
    <div className={cn('record-header flex flex-shrink-0 border-b border-border bg-panel', FACEPLATE_BEVEL)}>
      <div className="flex min-w-0 flex-1 flex-col justify-end">
        <div ref={leftColumnRef} className={cn('pl-4 pr-4 pt-4', hasViewTabs ? 'pb-0' : 'pb-4')}>
          <PageHeader
            className="p-0 min-[900px]:px-0"
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
          />
          {onViewChange && (
            // The tabs' underline lands on the band's bottom rule, so they draw none of
            // their own; -ml-3 cancels the first tab's padding so its label sits on the edge.
            <RecordViewTabs
              view={view}
              onViewChange={onViewChange}
              agendaAvailable={!!activeRecordingMeetingId}
              className="-ml-3 mt-2.5 border-b-0"
            />
          )}
        </div>
      </div>
      {meters}
    </div>
  );
}
