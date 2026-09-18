import { X } from 'lucide-react';
import { cn } from '@/lib/utils';

/**
 * specs/0061 W4 (task 3) — shown above the transcript list while it's filtered to
 * one speaker (clicked in the channel strip). Keeps the chip markup out of
 * TranscriptPanel itself.
 */
export function SpeakerFilterChip({
  displayName,
  onClear,
  className,
}: {
  /** The filtered speaker's display name (e.g. "Tomas"). */
  displayName: string;
  onClear?: () => void;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'inline-flex items-center gap-1.5 rounded-[3px] border border-border bg-card px-2 py-0.5 text-xs',
        className,
      )}
    >
      <span className="text-foreground">Showing: {displayName}</span>
      <button
        type="button"
        onClick={onClear}
        aria-label="Clear speaker filter"
        className="rounded-[2px] text-muted-foreground hover:text-foreground"
      >
        <X size={12} />
      </button>
    </div>
  );
}
