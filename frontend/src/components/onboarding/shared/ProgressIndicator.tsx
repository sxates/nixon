import React from 'react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface ProgressIndicatorProps {
  current: number;
  total: number;
  onStepClick?: (step: number) => void;
}

/**
 * Onboarding position markers: one engraved bar per step — done (`bg-success`), current
 * (`bg-brand`), pending (`bg-border`). No per-step icons (docs/design-system.md §Icons:
 * onboarding steps carry no decorative glyphs); a completed bar is clickable to go back.
 */
export function ProgressIndicator({ current, total, onStepClick }: ProgressIndicatorProps) {
  const visibleSteps = Array.from({ length: total }, (_, i) => i + 1);

  return (
    <div className="mb-8">
      <div className="flex items-center justify-center gap-2">
        {visibleSteps.map((step, index) => {
          const isActive = step === current;
          const isCompleted = step < current;
          const isClickable = isCompleted && onStepClick;

          return (
            <React.Fragment key={step}>
              <Button
                variant="ghost"
                size="xs"
                onClick={() => isClickable && onStepClick(step)}
                disabled={!isClickable}
                aria-label={`Step ${step} of ${total}${isCompleted ? ' — completed' : isActive ? ' — current' : ''}`}
                aria-current={isActive ? 'step' : undefined}
                // The bar is the control; drop the button's own padding and keep a
                // non-clickable marker at full strength (not the disabled 50%).
                className={cn('h-auto px-1 py-1.5 disabled:opacity-100', isClickable ? 'cursor-pointer' : 'cursor-default')}
              >
                <span
                  className={cn(
                    'h-2 w-6 rounded-sm transition-colors duration-300',
                    isCompleted ? 'bg-success' : isActive ? 'bg-brand' : 'bg-border',
                  )}
                />
              </Button>

              {index < visibleSteps.length - 1 && (
                <div
                  className={cn('h-0.5 w-6 transition-colors duration-300', isCompleted ? 'bg-success' : 'bg-border')}
                />
              )}
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}
