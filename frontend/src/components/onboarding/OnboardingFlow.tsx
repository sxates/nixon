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
  // `null` = platform detection still in flight. Steps 4/5 branch on
  // platform (Permissions is mac-only; step 4 means Calendar on non-mac,
  // Permissions on mac), so until detection resolves, NEITHER branch may
  // render — see the `isMac === true` / `isMac === false` checks below
  // (specs/0061 W1 Task 3 fix, controller ruling R15). Rendering either one
  // on a guess would let a real Mac user land on the wrong step 4 content —
  // both this step's actions (Finish Setup / Skip) complete onboarding, so a
  // mistimed click would silently skip requesting mic/system-audio
  // permissions.
  const [isMac, setIsMac] = React.useState<boolean | null>(null);

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
      {currentStep === 4 && isMac === true && <PermissionsStep />}
      {currentStep === 5 && isMac === true && <CalendarStep />}
      {currentStep === 4 && isMac === false && <CalendarStep />}
    </div>
  );
}
