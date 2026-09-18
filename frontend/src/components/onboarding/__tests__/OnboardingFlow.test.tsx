import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// specs/0061 W1 Task 3 fix (controller ruling R15) — before this task, the
// step-4 slot only ever branched on `isMac` being truthy (Permissions,
// macOS-only); the non-mac branch didn't exist, so during the async
// platform-detection window a real Mac user just saw a blank step 4.
// Adding the Calendar step's `isMac === false` branch made that same window
// reachable for a REAL Mac user to see (and act on) the WRONG step: both
// "Finish Setup" and "Skip for now" on the Calendar step complete onboarding
// outright, skipping Permissions (mic/system-audio) entirely. `isMac` is now
// `boolean | null`, and every platform-gated branch requires a resolved
// (non-null) value, restoring the original "render nothing while detecting"
// safety.

const useOnboardingMock = vi.fn();
vi.mock('@/contexts/OnboardingContext', () => ({
  useOnboarding: () => useOnboardingMock(),
}));

// Stub the step components so this test exercises ONLY OnboardingFlow's own
// step/platform gating — not each step's own internals/dependencies.
vi.mock('@/components/onboarding/steps', () => ({
  WelcomeStep: () => <div>WelcomeStepStub</div>,
  SetupOverviewStep: () => <div>SetupOverviewStepStub</div>,
  DownloadProgressStep: () => <div>DownloadProgressStepStub</div>,
  PermissionsStep: () => <div>PermissionsStepStub</div>,
  CalendarStep: () => <div>CalendarStepStub</div>,
}));

const { getMockPlatform, setMockPlatform } = vi.hoisted(() => {
  let mockPlatform = 'macos';
  return {
    getMockPlatform: () => mockPlatform,
    setMockPlatform: (p: string) => {
      mockPlatform = p;
    },
  };
});
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => getMockPlatform() }));

import { OnboardingFlow } from '@/components/onboarding/OnboardingFlow';

beforeEach(() => {
  useOnboardingMock.mockReset();
  setMockPlatform('macos');
});

describe('OnboardingFlow — platform-gated steps stay blank until detection resolves', () => {
  it('renders NOTHING for step 4 while platform detection is still in flight, even for a real Mac', () => {
    // The dangerous case: this WILL resolve to macOS, but hasn't yet.
    setMockPlatform('macos');
    useOnboardingMock.mockReturnValue({ currentStep: 4 });
    render(<OnboardingFlow onComplete={() => {}} />);

    // Checked synchronously, before the platform-detecting effect's
    // `await import(...)` has had a chance to resolve: neither the mac
    // (Permissions) nor non-mac (Calendar) step-4 branch may render yet.
    expect(screen.queryByText('PermissionsStepStub')).toBeNull();
    expect(screen.queryByText('CalendarStepStub')).toBeNull();
  });

  it('renders NOTHING for step 5 while platform detection is still in flight', () => {
    setMockPlatform('macos');
    useOnboardingMock.mockReturnValue({ currentStep: 5 });
    render(<OnboardingFlow onComplete={() => {}} />);

    expect(screen.queryByText('CalendarStepStub')).toBeNull();
  });

  it('shows Permissions at step 4 on macOS once detection resolves', async () => {
    setMockPlatform('macos');
    useOnboardingMock.mockReturnValue({ currentStep: 4 });
    render(<OnboardingFlow onComplete={() => {}} />);

    expect(await screen.findByText('PermissionsStepStub')).toBeInTheDocument();
    expect(screen.queryByText('CalendarStepStub')).toBeNull();
  });

  it('shows Calendar at step 5 on macOS once detection resolves', async () => {
    setMockPlatform('macos');
    useOnboardingMock.mockReturnValue({ currentStep: 5 });
    render(<OnboardingFlow onComplete={() => {}} />);

    expect(await screen.findByText('CalendarStepStub')).toBeInTheDocument();
  });

  it('shows Calendar (not Permissions) at step 4 on non-macOS once detection resolves', async () => {
    setMockPlatform('linux');
    useOnboardingMock.mockReturnValue({ currentStep: 4 });
    render(<OnboardingFlow onComplete={() => {}} />);

    expect(await screen.findByText('CalendarStepStub')).toBeInTheDocument();
    expect(screen.queryByText('PermissionsStepStub')).toBeNull();
  });
});
