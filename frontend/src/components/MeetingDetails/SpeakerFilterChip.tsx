import { X } from '@/components/ui/icons';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
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
        'inline-flex items-center gap-1.5 rounded-md border border-border bg-card py-0.5 pl-2 pr-0.5 text-xs',
        className,
      )}
    >
      <span className="text-foreground">Showing: {displayName}</span>
      <IconButton
        variant="ghost"
        label="Clear speaker filter"
        onClick={onClear}
        icon={<DeckIcon icon={X} size={12} />}
        className="h-5 w-5"
      />
    </div>
  );
}
