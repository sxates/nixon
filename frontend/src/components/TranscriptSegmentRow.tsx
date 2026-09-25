'use client';

import { memo, useState } from "react";
import { Check, Pencil } from "lucide-react";
import { ConfidenceIndicator } from "./ConfidenceIndicator";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";
import { MeetingAttendee, Person } from "@/types";
import { speakerColorClass, speakerBgClass } from "@/lib/speaker-colors";
import { cn } from "@/lib/utils";
import { InlineSpeakerAssign } from "./MeetingDetails/InlineSpeakerAssign";
import type { OwnerActionContext } from "./MeetingDetails/SpeakerOwnerAction";
import { SegmentSpeakerMenu } from "./MeetingDetails/SegmentSpeakerMenu";
import { SegmentTextEditor } from "./MeetingDetails/SegmentTextEditor";

/** specs/0019 WS2.1 — wiring that lets a transcript line reassign its speaker inline.
 *  Provided by the meeting-details panel (not during live recording); absent => the
 *  speaker name renders as static text exactly as before. */
export interface InlineSpeakerAssignment {
  attendees: MeetingAttendee[];
  people: Person[];
  onAssignAttendee: (
    speakerKey: string,
    attendee: { name: string; email: string },
  ) => Promise<void>;
  onAssignPerson: (speakerKey: string, person: Person) => Promise<void>;
  /** specs/0019 WS2.3 — the meeting's speakers, to move a single line to another of
   *  them (the per-segment "split" fix), plus the reassign action. Resolves `true` on
   *  success, `false` on failure (the caller surfaces the toast); the optimistic
   *  overlay + revert is owned by this view (specs/0041 WS7.2). */
  speakers: { speakerKey: string; displayName: string }[];
  onReassignSegment: (transcriptId: string, speakerKey: string) => Promise<boolean>;
  /** specs/0039 WS2 — bulk span reassignment: move a contiguous run of lines to one
   *  EXISTING speaker in a single write. Resolves `true` on success, `false` on failure
   *  (the caller surfaces the toast). The caller reconciles in the BACKGROUND (specs/0041
   *  WS7.2 — the refetch preserves the pagination window + scroll position); this view's
   *  optimistic overlay covers each line until the reconciled data lands. */
  onReassignSegments: (
    transcriptIds: string[],
    speakerKey: string,
  ) => Promise<boolean>;
  /** specs/0039 WS2 — mint a brand-new meeting speaker (`manual_<uuid>`) for a span the
   *  clusterer missed, returning its stable key + name. Resolves `null` on failure
   *  (the caller toasts). No embedding/voiceprint is attached, so the new key is a
   *  plain reassignment target — no gallery affordances are offered for it. */
  onCreateSpeaker: (
    displayName: string,
  ) => Promise<{ speakerKey: string; displayName: string } | null>;
  /** specs/0078 — "This is me" / "This isn't me" in the name's popover. */
  owner?: OwnerActionContext;
}

// Helper function to format seconds as recording-relative time [MM:SS]
function formatRecordingTime(seconds: number | undefined): string {
    if (seconds === undefined) return '[--:--]';

    const totalSeconds = Math.floor(seconds);
    const minutes = Math.floor(totalSeconds / 60);
    const secs = totalSeconds % 60;

    return `[${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
}

// Helper function to remove filler words and repetitions
function cleanStopWords(text: string): string {
    const stopWords = ['uh', 'um', 'er', 'ah', 'hmm', 'hm', 'eh', 'oh'];

    let cleanedText = text;
    stopWords.forEach(word => {
        const pattern = new RegExp(`\\b${word}\\b[,\\s]*`, 'gi');
        cleanedText = cleanedText.replace(pattern, ' ');
    });

    return cleanedText.replace(/\s+/g, ' ').trim();
}

// Derive up to 2 uppercase initials from a speaker label.
// "Sarah Chen" -> "SC", "You" -> "Y", "alex" -> "A".
function speakerInitials(speakerName: string): string {
    const parts = speakerName.trim().split(/\s+/).filter(Boolean);
    if (parts.length === 0) return '';
    if (parts.length === 1) return parts[0].charAt(0).toUpperCase();
    return (parts[0].charAt(0) + parts[parts.length - 1].charAt(0)).toUpperCase();
}

// Speaker avatar: colored circle with initials when diarized, neutral dot otherwise.
function SpeakerAvatar({
    speaker,
    speakerName,
}: {
    speaker?: string | null;
    speakerName?: string | null;
}) {
    const initials = speakerName ? speakerInitials(speakerName) : '';

    if (!speakerName || !initials) {
        // Undiarized: neutral avatar with a subtle person dot (no "undefined").
        return (
            <div
                className="w-7 h-7 flex-shrink-0 rounded-full bg-muted flex items-center justify-center"
                aria-hidden="true"
            >
                <div className="w-2 h-2 rounded-full bg-muted-foreground/50" />
            </div>
        );
    }

    return (
        <div
            className={`w-7 h-7 flex-shrink-0 rounded-full flex items-center justify-center text-[11px] font-semibold ${speakerBgClass(speaker) === 'bg-muted' ? 'text-foreground' : 'text-background'} ${speakerBgClass(speaker)}`}
            aria-hidden="true"
        >
            {initials}
        </div>
    );
}

// specs/0039 WS2 — per-line selection checkbox for span reassignment. Hidden until the
// row is hovered (like the per-line split menu) unless a selection is active or this
// line is selected, so it doesn't clutter the transcript in the common read case.
// Shift-click extends a contiguous range from the anchor (index-based, so it survives
// virtualization — the range is computed from the segments array, not DOM order).
const SelectionCheckbox = memo(function SelectionCheckbox({
    id,
    index,
    selected,
    active,
    onToggle,
}: {
    id: string;
    index: number;
    selected: boolean;
    active: boolean;
    onToggle: (id: string, index: number, shiftKey: boolean) => void;
}) {
    return (
        <div className="flex w-4 flex-shrink-0 items-center pt-1.5">
            <button
                type="button"
                role="checkbox"
                aria-checked={selected}
                aria-label={selected ? 'Deselect this line' : 'Select this line'}
                onClick={(e) => {
                    // Prevent the click from starting a text selection on shift-click.
                    e.preventDefault();
                    onToggle(id, index, e.shiftKey);
                }}
                className={cn(
                    'flex h-4 w-4 items-center justify-center rounded border transition-opacity',
                    selected
                        ? 'border-brand bg-brand text-brand-foreground'
                        : 'border-muted-foreground/40 bg-transparent hover:border-brand',
                    selected || active
                        ? 'opacity-100'
                        : 'opacity-0 focus-visible:opacity-100 group-hover/segment:opacity-100',
                )}
            >
                {selected && <Check size={11} strokeWidth={3} />}
            </button>
        </div>
    );
});

// Memoized transcript segment component
export const TranscriptSegment = memo(function TranscriptSegment({
    id,
    index,
    timestamp,
    text,
    confidence,
    isStreaming,
    showConfidence,
    speaker,
    speakerName,
    assignment,
    selectable,
    selected,
    selectionActive,
    onToggleSelect,
    userEdited,
    continuesRun,
    onEditText,
}: {
    id: string;
    index: number;
    timestamp: number;
    text: string;
    confidence?: number;
    isStreaming: boolean;
    showConfidence: boolean;
    // Speaker diarization (specs/0010). NULL/absent => render exactly as before (no label).
    speaker?: string | null;
    speakerName?: string | null;
    // specs/0019 WS2.1 — inline assignment wiring; absent => name is static text.
    assignment?: InlineSpeakerAssignment;
    // specs/0039 WS2 — span selection (meeting-details only). Absent => no checkbox.
    selectable?: boolean;
    selected?: boolean;
    selectionActive?: boolean;
    onToggleSelect?: (id: string, index: number, shiftKey: boolean) => void;
    // specs/0061 W5 (task 5) — true once this line's text has been manually
    // corrected (via onEditText below); shows the "edited" mark.
    userEdited?: boolean;
    /**
     * This line continues the previous line's speaker run (owner feedback 2026-09-21 — the
     * transcript was one tall bordered block per line, each repeating an avatar and a name
     * that had not changed; three consecutive "YOU" lines cost three headers and three
     * dividers). When true the row drops its avatar and name, keeps the avatar's gutter so
     * the text stays aligned, and joins the block above with no divider.
     *
     * Computed by the caller from the SAME array it renders, so an optimistic speaker
     * reassignment regroups the run immediately rather than at the next refetch. Absent =>
     * every line is its own run, which is exactly the pre-2026-09-21 rendering (and the
     * right answer for an undiarized transcript, where there is no speaker to group by).
     */
    continuesRun?: boolean;
    /** specs/0061 W5 — save this line's RAW edited text. Absent (recording, or no
     *  meetingId) => no pencil, no inline edit affordance. Resolves `true` on success
     *  (the editor closes and the row shows the new text) or `false` on failure — on
     *  failure the editor STAYS OPEN with what the user typed (specs/0061 W5 review,
     *  R40: a failed save must not throw away a hand-typed correction); the caller
     *  (TranscriptPanel) also surfaces a toast, but this row's own inline message is
     *  the primary signal. */
    onEditText?: (id: string, text: string) => Promise<boolean>;
}) {
    const [editing, setEditing] = useState(false);
    // specs/0061 W5 review (Important 1, R40) — a failed save must not throw away
    // what the user typed: the editor stays OPEN (SegmentTextEditor keeps its own
    // `value` because it isn't unmounted) and this inline message is the primary
    // failure signal, not the transient toast alone.
    const [saveError, setSaveError] = useState<string | null>(null);
    const displayText = cleanStopWords(text) || (text.trim() === '' ? '[Silence]' : text);

    const handleSave = async (newText: string): Promise<boolean> => {
        if (!onEditText) return false;
        setSaveError(null);
        const ok = await onEditText(id, newText);
        if (ok) {
            setEditing(false);
        } else {
            setSaveError("Could not save this edit — try again.");
        }
        return ok;
    };

    const handleCancel = () => {
        setSaveError(null);
        setEditing(false);
    };

    return (
        // A run reads as one block: the divider and the breathing room go on the run's
        // FIRST line as a top rule (suppressed at index 0), and continuation lines sit
        // tight beneath it with nothing between them. Doing it this way rather than
        // bottom-bordering the last line of a run means a row never has to know what
        // follows it — which it can't, in a windowed list.
        <div
            id={`segment-${id}`}
            // Marks the line that carries a run's avatar + name. Read by tests to assert
            // which speaker governs which lines, and a useful handle when debugging a
            // transcript whose runs look wrong.
            data-run-start={continuesRun ? undefined : 'true'}
            className={cn(
                'group/segment',
                continuesRun
                    ? 'mt-1'
                    : index > 0
                      ? 'mt-3 border-t border-border/60 pt-3'
                      : '',
            )}
        >
            {/* Run header: avatar + name, once per run. */}
            {!continuesRun && (
                <div className="mb-1 flex items-center gap-[11px]">
                    {/* Keeps the header's avatar aligned with the checkbox column below. */}
                    {selectable && onToggleSelect && <div className="w-4 flex-shrink-0" aria-hidden="true" />}
                    <SpeakerAvatar speaker={speaker} speakerName={speakerName} />
                    {speakerName && speaker && assignment ? (
                        // WS2.1: name a speaker right from the transcript line.
                        <InlineSpeakerAssign
                            speakerKey={speaker}
                            speakerName={speakerName}
                            attendees={assignment.attendees}
                            people={assignment.people}
                            onAssignAttendee={assignment.onAssignAttendee}
                            onAssignPerson={assignment.onAssignPerson}
                            owner={assignment.owner}
                        />
                    ) : speakerName ? (
                        <span className={`u-typed font-bold uppercase text-[12px] tracking-[0.02em] whitespace-nowrap ${speakerColorClass(speaker)}`}>
                            {speakerName}
                        </span>
                    ) : (
                        <span className="u-typed font-bold uppercase text-[12px] tracking-[0.02em] whitespace-nowrap text-muted-foreground">
                            Speaker
                        </span>
                    )}
                </div>
            )}

            <div className="flex items-start gap-[11px]">
                {selectable && onToggleSelect && (
                    <SelectionCheckbox
                        id={id}
                        index={index}
                        selected={!!selected}
                        active={!!selectionActive}
                        onToggle={onToggleSelect}
                    />
                )}
                {/* The avatar's gutter, held open on every line so a run's text forms one
                    left-aligned column whether or not the line carries a header. */}
                <div className="w-7 flex-shrink-0" aria-hidden="true" />
                <div className="min-w-0 flex-1">
                    {editing ? (
                        <>
                            <SegmentTextEditor
                                initialText={text}
                                onSave={handleSave}
                                onCancel={handleCancel}
                            />
                            {saveError && (
                                <p className="mt-1 text-[11px] text-destructive">{saveError}</p>
                            )}
                        </>
                    ) : isStreaming ? (
                        <div className="bg-muted border border-border rounded-lg px-3 py-2">
                            <p className="u-typed">{displayText}</p>
                        </div>
                    ) : (
                        <p className="u-typed">{displayText}</p>
                    )}
                </div>
                {/* Per-LINE affordances and this line's timestamp, pinned to the right edge
                    (owner feedback 2026-09-21: "with time stamps off to the right"). The
                    hover controls are `opacity-0`, not `hidden`, so they hold their space
                    and the timestamp column never shifts as the pointer moves down the
                    transcript. Hidden while editing — the editor owns the row. */}
                {!editing && (
                    <div className="flex flex-shrink-0 items-center gap-0.5 pt-px">
                        {userEdited && (
                            <span
                                className="rounded-[2px] border border-border px-1 py-px text-[10px] font-medium text-engrave"
                                title="Manually edited"
                            >
                                edited
                            </span>
                        )}
                        {/* WS2.3: move just this line to another speaker (sticky split). */}
                        {speaker && assignment && (
                            <SegmentSpeakerMenu
                                transcriptId={id}
                                currentSpeakerKey={speaker}
                                speakers={assignment.speakers}
                                onReassign={assignment.onReassignSegment}
                            />
                        )}
                        {/* specs/0061 W5 — hover pencil next to the split icon: correct this
                            line's text in place. Independent of speaker assignment. */}
                        {onEditText && (
                            <button
                                type="button"
                                onClick={() => {
                                    setSaveError(null);
                                    setEditing(true);
                                }}
                                className="rounded-[2px] p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover/segment:opacity-100"
                                title="Edit this line's text"
                                aria-label="Edit this line's text"
                            >
                                <Pencil size={12} />
                            </button>
                        )}
                        <Tooltip>
                            <TooltipTrigger asChild>
                                <span className="ml-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
                                    {formatRecordingTime(timestamp)}
                                </span>
                            </TooltipTrigger>
                            <TooltipContent>
                                {confidence !== undefined && showConfidence && (
                                    <ConfidenceIndicator confidence={confidence} showIndicator={showConfidence} />
                                )}
                            </TooltipContent>
                        </Tooltip>
                    </div>
                )}
            </div>
        </div>
    );
});
