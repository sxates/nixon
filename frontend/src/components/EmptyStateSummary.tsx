'use client';

import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState } from '@/components/ui/empty-state';
import { Sparkles } from '@/components/ui/icons';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

interface EmptyStateSummaryProps {
  onGenerate: () => void;
  hasModel: boolean;
  isGenerating?: boolean;
}

/**
 * The Summary tab before a summary exists. The CTA is the amber primary (`brand`) with the
 * Sparkles glyph that marks an AI action — never the REC red, which means recording only
 * (docs/design-system.md §Colour). No icon disc above the heading (§Icons).
 */
export function EmptyStateSummary({ onGenerate, hasModel, isGenerating = false }: EmptyStateSummaryProps) {
  return (
    <EmptyState
      titleAs="h3"
      className="h-full animate-fade-in p-8"
      title="No Summary Generated Yet"
      body="Generate an AI-powered summary of your meeting transcript to get key points, action items, and decisions."
      action={
        <div className="flex flex-col items-center gap-3">
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <div>
                  <Button variant="brand" onClick={onGenerate} disabled={!hasModel || isGenerating}>
                    <DeckIcon icon={Sparkles} size={16} />
                    {isGenerating ? 'Generating...' : 'Generate Summary'}
                  </Button>
                </div>
              </TooltipTrigger>
              {!hasModel && (
                <TooltipContent>
                  <p>Please select a model in Settings first</p>
                </TooltipContent>
              )}
            </Tooltip>
          </TooltipProvider>
          {!hasModel && <p className="text-xs text-brand">Please select a model in Settings first</p>}
        </div>
      }
    />
  );
}
