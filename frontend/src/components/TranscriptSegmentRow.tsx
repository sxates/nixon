'use client';

import { memo, useState } from "react";
import { Check, Pencil } from "lucide-react";
import { ConfidenceIndicator } from "./ConfidenceIndicator";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";
import { MeetingAttendee, Person } from "@/types";
import { speakerColorClass, speakerBgClass } from "@/lib/speaker-colors";
import { cn } from "@/lib/utils";
import { InlineSpeakerAssign } from "./MeetingDetails/InlineSpeakerAssign";
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
                        : 'opacity-0 focus:opacity-100 group-hover/segment:opacity-100',
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
    /** specs/0061 W5 — save this line's RAW edited text. Absent (recording, or no
     *  meetingId) => no pencil, no inline edit affordance. Same success/failure
     *  contract as onReassignSegment; the optimistic overlay + revert is owned by
     *  the parent view. */
    onEditText?: (id: string, text: string) => Promise<boolean>;
}) {
    const [editing, setEditing] = useState(false);
    const displayText = cleanStopWords(text) || (text.trim() === '' ? '[Silence]' : text);

    const handleSave = async (newText: string): Promise<boolean> => {
        if (!onEditText) return false;
        const ok = await onEditText(id, newText);
        setEditing(false);
        return ok;
    };

    return (
        <div id={`segment-${id}`} className="group/segment mb-3 border-b border-border/60 pb-3">
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
                <SpeakerAvatar speaker={speaker} speakerName={speakerName} />
                <div className="flex-1 min-w-0">
                    <div className="flex items-baseline gap-2 mb-0.5">
                        {speakerName && speaker && assignment ? (
                            // WS2.1: name a speaker right from the transcript line.
                            <InlineSpeakerAssign
                                speakerKey={speaker}
                                speakerName={speakerName}
                                attendees={assignment.attendees}
                                people={assignment.people}
                                onAssignAttendee={assignment.onAssignAttendee}
                                onAssignPerson={assignment.onAssignPerson}
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
                        <Tooltip>
                            <TooltipTrigger asChild>
                                <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
                                    {formatRecordingTime(timestamp)}
                                </span>
                            </TooltipTrigger>
                            <TooltipContent>
                                {confidence !== undefined && showConfidence && (
                                    <ConfidenceIndicator confidence={confidence} showIndicator={showConfidence} />
                                )}
                            </TooltipContent>
                        </Tooltip>
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
                        {onEditText && !editing && (
                            <button
                                type="button"
                                onClick={() => setEditing(true)}
                                className="ml-0.5 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus:opacity-100 group-hover/segment:opacity-100"
                                title="Edit this line's text"
                                aria-label="Edit this line's text"
                            >
                                <Pencil size={12} />
                            </button>
                        )}
                        {userEdited && (
                            <span
                                className="rounded-[2px] border border-border px-1 py-px text-[10px] font-medium text-engrave"
                                title="Manually edited"
                            >
                                edited
                            </span>
                        )}
                    </div>
                    {editing ? (
                        <SegmentTextEditor
                            initialText={text}
                            onSave={handleSave}
                            onCancel={() => setEditing(false)}
                        />
                    ) : isStreaming ? (
                        <div className="bg-muted border border-border rounded-lg px-3 py-2">
                            <p className="u-typed">{displayText}</p>
                        </div>
                    ) : (
                        <p className="u-typed">{displayText}</p>
                    )}
                </div>
            </div>
        </div>
    );
});
