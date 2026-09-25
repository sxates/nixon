import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// specs/0061 W1 Task 2 — AppShell holds the showOnboarding ternary extracted
// from RootLayout. The whole point of the extraction is that
// DownloadProgressToastProvider (the top-right download toasts) mounts ONLY
// in the post-onboarding branch — during onboarding, DownloadProgressStep's
// own in-page cards already show progress, so the toast provider would
// duplicate that feedback. These assertions pin both sides of that
// conditional (not just "the mock was called once"): the toast provider and
// the rest of the post-onboarding shell render when showOnboarding is false
// and are absent when it's true, while OnboardingFlow is the mirror image.

const {
  downloadProgressToastProviderMock,
  onboardingFlowMock,
  sidebarMock,
  transportRailMock,
  mainContentMock,
  commandPaletteMock,
} = vi.hoisted(() => ({
  downloadProgressToastProviderMock: vi.fn(() => null),
  onboardingFlowMock: vi.fn(() => null),
  sidebarMock: vi.fn(() => null),
  transportRailMock: vi.fn(() => null),
  mainContentMock: vi.fn(() => null),
  commandPaletteMock: vi.fn(() => null),
}));

vi.mock('@/components/shared/DownloadProgressToast', () => ({
  DownloadProgressToastProvider: downloadProgressToastProviderMock,
}));
vi.mock('@/components/onboarding', () => ({
  OnboardingFlow: onboardingFlowMock,
}));
vi.mock('@/components/Sidebar', () => ({ default: sidebarMock }));
vi.mock('@/components/Transport/TransportRail', () => ({ TransportRail: transportRailMock }));
vi.mock('@/components/MainContent', () => ({ default: mainContentMock }));
vi.mock('@/components/CommandPalette', () => ({ default: commandPaletteMock }));

vi.mock('@/components/ResumeRecordingPrompt', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/MeetingAutoDetect', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/NotificationPermissionBootstrap', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/Calendar/CalendarAlerts', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/People/VoiceprintRetractionListener', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/PermissionsModal', () => ({ default: vi.fn(() => null) }));
vi.mock('@/components/Updates/UpdatedNotice', () => ({ UpdatedNotice: vi.fn(() => null) }));
vi.mock('@/components/RecordingsMoveWatcher', () => ({ RecordingsMoveWatcher: vi.fn(() => null) }));
vi.mock('@/contexts/QueueViewContext', () => ({
  QueueViewProvider: ({ children }: { children: React.ReactNode }) => children,
}));
vi.mock('@/contexts/LlmActivityProvider', () => ({
  LlmActivityProvider: ({ children }: { children: React.ReactNode }) => children,
}));

import { AppShell } from '@/app/_components/AppShell';

const onOnboardingComplete = vi.fn();

beforeEach(() => {
  downloadProgressToastProviderMock.mockClear();
  onboardingFlowMock.mockClear();
  sidebarMock.mockClear();
  transportRailMock.mockClear();
  mainContentMock.mockClear();
  commandPaletteMock.mockClear();
});

describe('AppShell', () => {
  it('does not mount the download-progress toast provider (or the rest of the post-onboarding shell) while onboarding is showing', () => {
    render(
      <AppShell showOnboarding onOnboardingComplete={onOnboardingComplete}>
        <div>app content</div>
      </AppShell>
    );

    expect(onboardingFlowMock).toHaveBeenCalled();
    expect(downloadProgressToastProviderMock).not.toHaveBeenCalled();
    expect(sidebarMock).not.toHaveBeenCalled();
    expect(transportRailMock).not.toHaveBeenCalled();
    expect(mainContentMock).not.toHaveBeenCalled();
    expect(commandPaletteMock).not.toHaveBeenCalled();
  });

  it('mounts the download-progress toast provider (and the rest of the shell) once onboarding is done', () => {
    render(
      <AppShell showOnboarding={false} onOnboardingComplete={onOnboardingComplete}>
        <div>app content</div>
      </AppShell>
    );

    expect(downloadProgressToastProviderMock).toHaveBeenCalledTimes(1);
    expect(sidebarMock).toHaveBeenCalled();
    expect(transportRailMock).toHaveBeenCalled();
    expect(mainContentMock).toHaveBeenCalled();
    expect(commandPaletteMock).toHaveBeenCalled();
    expect(onboardingFlowMock).not.toHaveBeenCalled();
  });
});
