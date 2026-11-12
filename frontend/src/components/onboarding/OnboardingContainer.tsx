import React from 'react';
import { ChevronLeft, ChevronRight } from '@/components/ui/icons';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { cn } from '@/lib/utils';
import { ProgressIndicator } from './shared/ProgressIndicator';
import { useOnboarding } from '@/contexts/OnboardingContext';
import type { OnboardingContainerProps } from '@/types/onboarding';

export function OnboardingContainer({
  title,
  description,
  children,
  step,
  totalSteps = 5,
  stepOffset = 0,
  hideProgress = false,
  className,
  showNavigation = false,
  onNext,
  onPrevious,
  canGoNext = true,
  canGoPrevious = true,
}: OnboardingContainerProps) {
  const { goToStep, goPrevious, goNext } = useOnboarding();

  const handlePrevious = () => {
    if (onPrevious) {
      onPrevious();
    } else {
      goPrevious();
    }
  };

  const handleNext = () => {
    if (onNext) {
      onNext();
    } else {
      goNext();
    }
  };

  const handleStepClick = (s: number) => {
    goToStep(s + stepOffset);
  };

  return (
    <div className="fixed inset-0 bg-background flex items-center justify-center z-50 overflow-hidden">
      <div className={cn('w-full max-w-2xl h-full max-h-screen flex flex-col px-6 py-6', className)}>
        {/* Progress Indicator with Navigation - Fixed */}
        {step && !hideProgress && (
          <div className="mb-2 relative flex-shrink-0">
            {/* Navigation Buttons */}
            {showNavigation && (
              <div className="absolute top-1/2 -translate-y-1/2 left-0 right-0 flex justify-between pointer-events-none">
                <IconButton
                  label="Previous step"
                  size="md"
                  icon={<DeckIcon icon={ChevronLeft} size={16} />}
                  onClick={handlePrevious}
                  disabled={!canGoPrevious || step === 1}
                  className={cn('pointer-events-auto text-foreground', (!canGoPrevious || step === 1) && 'invisible')}
                />

                <IconButton
                  label="Next step"
                  size="md"
                  icon={<DeckIcon icon={ChevronRight} size={16} />}
                  onClick={handleNext}
                  disabled={!canGoNext || step === totalSteps}
                  className={cn('pointer-events-auto text-foreground', (!canGoNext || step === totalSteps) && 'invisible')}
                />
              </div>
            )}

            {/* Progress Indicator */}
            <ProgressIndicator current={step} total={totalSteps} onStepClick={handleStepClick} />
          </div>
        )}

        {/* Header - Fixed */}
        <div className="mb-4 text-center space-y-3 flex-shrink-0">
          <h1 className="font-display text-display font-semibold text-foreground animate-fade-in">{title}</h1>
          {description && (
            <p className="text-base text-muted-foreground max-w-md mx-auto animate-fade-in">
              {description}
            </p>
          )}
        </div>

        {/* Content - Scrollable */}
        <div className="flex-1 overflow-y-auto pr-2">
          <div className="space-y-6">{children}</div>
        </div>
      </div>
    </div>
  );
}
