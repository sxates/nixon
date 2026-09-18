import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Calendar } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { PermissionRow } from '../shared';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { useGoogleCalendarConnect } from '@/hooks/useGoogleCalendarConnect';
import {
  getCalendarAccessStatus,
  requestCalendarAccess,
  type CalendarAccessStatus,
} from '@/lib/calendar';
import type { PermissionStatus } from '@/types/onboarding';

/** Map EventKit's status onto the shared PermissionRow status. */
function mapEventKitStatus(status: CalendarAccessStatus | null): PermissionStatus {
  if (status === null) return 'checking';
  if (status === 'authorized') return 'authorized';
  if (status === 'denied' || status === 'restricted') return 'denied';
  return 'not_determined';
}

/**
 * Final, skippable onboarding step (specs/0061 W1): offers both Google
 * Calendar and macOS Calendar so a first-run user is offered a calendar
 * connection before they ever see the Today view / prep briefs that depend
 * on one. Neither row is required — both "Finish Setup" and "Skip for now"
 * complete onboarding the same way.
 */
export function CalendarStep() {
  const { completeOnboarding } = useOnboarding();
  const { connecting, connect, status } = useGoogleCalendarConnect();

  // `null` = platform detection still in flight. Unlike `OnboardingFlow`'s gate
  // (which controls whether this step renders at all), this only controls the
  // step's OWN "N of N" counter — the counter must not flash a wrong total
  // (e.g. "4 of 4" on a Mac, which is really "5 of 5") while detection resolves
  // (specs/0061 final review).
  const [isMac, setIsMac] = useState<boolean | null>(null);
  const [eventKitStatus, setEventKitStatus] = useState<CalendarAccessStatus | null>(null);
  const [eventKitPending, setEventKitPending] = useState(false);

  useEffect(() => {
    const checkPlatform = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (_e) {
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };
    checkPlatform();
  }, []);

  useEffect(() => {
    void getCalendarAccessStatus().then(setEventKitStatus);
  }, []);

  const handleMacCalendarAction = async () => {
    if (eventKitStatus === 'denied' || eventKitStatus === 'restricted') {
      try {
        await invoke('open_system_settings');
      } catch {
        toast.error('Could not open System Settings', {
          description:
            'Please enable Calendar access in System Settings → Privacy & Security → Calendars.',
        });
      }
      return;
    }

    setEventKitPending(true);
    try {
      const granted = await requestCalendarAccess();
      setEventKitStatus(granted ? 'authorized' : 'denied');
    } finally {
      setEventKitPending(false);
    }
  };

  const handleFinish = async () => {
    try {
      await completeOnboarding();
      window.location.reload();
    } catch (error) {
      console.error('[CalendarStep] Failed to complete onboarding:', error);
    }
  };

  const handleSkip = async () => {
    await handleFinish();
  };

  // Hidden only when this build explicitly has no Google OAuth client id
  // baked in (`status.configured === false`) — not while status is still
  // loading (`null`), so the row doesn't flash in after the first fetch.
  const showGoogleRow = status?.configured !== false;
  // Hide the counter entirely until platform detection resolves, rather than
  // guessing and flashing the wrong total.
  const totalSteps = isMac === null ? undefined : isMac ? 5 : 4;

  return (
    <OnboardingContainer
      title="Connect a calendar (optional)"
      description="Nixon can show today's meetings and prepare briefs from your calendar."
      step={totalSteps}
      totalSteps={totalSteps}
    >
      <div className="max-w-lg mx-auto space-y-6">
        <div className="space-y-4">
          {showGoogleRow && (
            <div role="group" aria-label="Google Calendar">
              <PermissionRow
                icon={<Calendar className="w-5 h-5" />}
                title="Google Calendar"
                description="Read-only access to your Google account's events."
                status={
                  status?.connected ? 'authorized' : status === null ? 'checking' : 'not_determined'
                }
                isPending={connecting}
                onAction={() => {
                  void connect();
                }}
              />
            </div>
          )}

          <div role="group" aria-label="macOS Calendar">
            <PermissionRow
              icon={<Calendar className="w-5 h-5" />}
              title="macOS Calendar"
              description="Read your Mac's calendar to show today's meetings on-device."
              status={mapEventKitStatus(eventKitStatus)}
              isPending={eventKitPending}
              onAction={() => {
                void handleMacCalendarAction();
              }}
            />
          </div>
        </div>

        <div className="flex flex-col gap-3 pt-4">
          <Button onClick={() => void handleFinish()} className="w-full h-11">
            Finish Setup
          </Button>

          <button
            onClick={() => void handleSkip()}
            className="text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            Skip for now
          </button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
