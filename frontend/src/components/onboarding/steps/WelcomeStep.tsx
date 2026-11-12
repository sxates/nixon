import React from 'react';
import { Button } from '@/components/ui/button';
import { StatusDot } from '@/components/ui/status-dot';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';

export function WelcomeStep() {
  const { goNext } = useOnboarding();

  const features = [
    'Your data never leaves your device',
    'Intelligent summaries & insights',
    'Works offline, no cloud required',
  ];

  return (
    <OnboardingContainer
      title="Welcome to Nixon"
      description="Nixon records what you hear, on this Mac. Nothing is uploaded."
      step={1}
      hideProgress={true}
    >
      <div className="flex flex-col items-center space-y-10">
        {/* Divider */}
        <div className="w-16 h-px bg-border" />

        {/* Features Card */}
        <ul className="w-full max-w-md space-y-4 rounded-lg border border-border bg-card p-6 shadow-card">
          {features.map((feature) => (
            <li key={feature} className="flex items-center gap-3">
              <StatusDot tone="muted" size={6} />
              <p className="text-sm leading-relaxed text-foreground">{feature}</p>
            </li>
          ))}
        </ul>

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-3">
          <Button variant="brand" size="lg" className="w-full" onClick={goNext}>
            Get Started
          </Button>
          <p className="text-xs text-center text-muted-foreground">Takes less than 3 minutes</p>
        </div>
      </div>
    </OnboardingContainer>
  );
}
