'use client';

import { Check, ChevronDown } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  promptSourceLabel,
  type MeetingPromptState,
  type SummaryPrompt,
} from '@/lib/summary-prompts';

interface PromptPickerProps {
  prompts: SummaryPrompt[];
  promptState: MeetingPromptState | null;
  onPromptSelect: (promptId: string, promptName: string) => void;
  /** Open the one-off "Custom for this meeting…" dialog. */
  onCustomPrompt: () => void;
  onClearCustomPrompt: () => void;
  className?: string;
  title?: string;
}

/** The per-meeting summary prompt dropdown shared by the meeting toolbar and the record header. */
export function PromptPicker({
  prompts,
  promptState,
  onPromptSelect,
  onCustomPrompt,
  onClearCustomPrompt,
  className,
  title,
}: PromptPickerProps) {
  const label = promptSourceLabel(promptState);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="outline"
          size="xs"
          aria-label={`Summary prompt: ${label}`}
          title={title ?? `Summary prompt: ${label}`}
          className={className}
        >
          <span className="truncate">{label}</span>
          <ChevronDown size={14} className="flex-shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {prompts.map((prompt) => (
          <DropdownMenuItem
            key={prompt.id}
            onSelect={() => onPromptSelect(prompt.id, prompt.name)}
            className="flex items-center justify-between gap-2"
          >
            <span>{prompt.name}</span>
            {promptState?.prompt_id === prompt.id && <Check className="h-4 w-4 text-brand" />}
          </DropdownMenuItem>
        ))}
        {prompts.length > 0 && <DropdownMenuSeparator />}
        <DropdownMenuItem onSelect={onCustomPrompt}>Custom for this meeting…</DropdownMenuItem>
        {promptState?.source === 'custom' && (
          <DropdownMenuItem onSelect={onClearCustomPrompt}>Remove custom prompt</DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
