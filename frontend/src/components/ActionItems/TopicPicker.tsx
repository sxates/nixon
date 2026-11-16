'use client';

/**
 * TopicPicker (specs/0090) — the explicit Topic control on an add surface (the To-do
 * page's add row, ⌘⇧T quick add): a small button showing the pending pick, "No Topic" by
 * default, that opens the open topics. It mirrors the `@topic` pick in the text field
 * (both write the same state), so either way of choosing shows here.
 */

import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { ChevronDown } from '@/components/ui/icons';
import { cn } from '@/lib/utils';
import type { MentionTopic } from './MentionInput';

const NONE = '';

interface TopicPickerProps {
  topics: MentionTopic[];
  value: MentionTopic | null;
  onChange: (topic: MentionTopic | null) => void;
  className?: string;
}

export function TopicPicker({ topics, value, onChange, className }: TopicPickerProps) {
  const label = value?.name ?? 'No Topic';
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          aria-label={`Topic: ${label}`}
          title={label}
          className={cn('h-8 max-w-[11rem] flex-shrink-0 gap-1 px-2.5 font-normal', !value && 'text-muted-foreground', className)}
        >
          <span className="truncate">{label}</span>
          <DeckIcon icon={ChevronDown} size={14} className="flex-shrink-0 opacity-60" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-72 overflow-y-auto">
        <DropdownMenuRadioGroup
          value={value?.id ?? NONE}
          onValueChange={(id) => onChange(id === NONE ? null : (topics.find((t) => t.id === id) ?? null))}
        >
          <DropdownMenuRadioItem value={NONE}>No Topic</DropdownMenuRadioItem>
          {topics.length > 0 ? <DropdownMenuSeparator /> : null}
          {topics.map((t) => (
            <DropdownMenuRadioItem key={t.id} value={t.id}>
              {t.name}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
