import React, { useEffect } from 'react';
import { useOnboarding } from '@/contexts/OnboardingContext';
import {
  WelcomeStep,
  PermissionsStep,
  DownloadProgressStep,
  SetupOverviewStep,
  CalendarStep,
} from './steps';

interface OnboardingFlowProps {
  onComplete: () => void;
}

export function OnboardingFlow({ onComplete: _onComplete }: OnboardingFlowProps) {
  const { currentStep } = useOnboarding();
  const [isMac, setIsMac] = React.useState(false);

  useEffect(() => {
    // Check if running on macOS
    const checkPlatform = async () => {
      try {
        // Dynamic import to avoid SSR issues if any
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (e) {
        console.error('Failed to detect platform:', e);
        // Fallback
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };
    checkPlatform();
  }, []);

  // Onboarding Flow (System-Recommended Models):
  // macOS:     1 Welcome, 2 Setup Overview, 3 Download Progress, 4 Permissions, 5 Calendar
  // Non-macOS: 1 Welcome, 2 Setup Overview, 3 Download Progress, 4 Calendar (no Permissions step)

  return (
    <div className="onboarding-flow">
      {currentStep === 1 && <WelcomeStep />}
      {currentStep === 2 && <SetupOverviewStep />}
      {currentStep === 3 && <DownloadProgressStep />}
      {currentStep === 4 && isMac && <PermissionsStep />}
      {currentStep === 5 && isMac && <CalendarStep />}
      {currentStep === 4 && !isMac && <CalendarStep />}
    </div>
  );
}
