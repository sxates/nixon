'use client';

/**
 * TopicChip (specs/0089 W4) — the topic a to-do is tagged to, as a quiet chip. On the list
 * row a click opens the topic; on an add surface (quick add) it shows the pending pick and
 * a click removes it.
 */

import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface TopicChipProps {
  name: string;
  onClick?: () => void;
  /** Accessible name; defaults to opening the topic. */
  label?: string;
  className?: string;
}

export function TopicChip({ name, onClick, label, className }: TopicChipProps) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="xs"
      aria-label={label ?? `Open topic ${name}`}
      title={name}
      onClick={(e) => {
        e.stopPropagation();
        onClick?.();
      }}
      className={cn(
        'h-5 min-w-0 max-w-full justify-start px-1.5 text-caption font-medium text-muted-foreground',
        className,
      )}
    >
      <span className="truncate">{name}</span>
    </Button>
  );
}
