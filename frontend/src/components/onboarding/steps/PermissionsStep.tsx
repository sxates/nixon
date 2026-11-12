import React, { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { PermissionRow } from '../shared';
import { openPrivacySettings } from '@/lib/system-settings';
import { useOnboarding } from '@/contexts/OnboardingContext';
import type { ProbeResult } from '@/types/onboarding';

export function PermissionsStep() {
  const { setPermissionStatus, setPermissionsSkipped, permissions, goNext } = useOnboarding();
  const [isPending, setIsPending] = useState(false);

  // Check permissions - only logs current state, doesn't auto-authorize
  // Actual permission checks are done via explicit user actions (clicking Enable)
  const checkPermissions = useCallback(async () => {
    console.log('[PermissionsStep] Current permission states:');
    console.log(`  - Microphone: ${permissions.microphone}`);
    console.log(`  - System Audio: ${permissions.systemAudio}`);
    // Don't auto-set permissions based on device availability
    // Permissions should only be set after explicit user action via Enable button
  }, [permissions.microphone, permissions.systemAudio]);

  // Check permissions on mount
  useEffect(() => {
    checkPermissions();
  }, [checkPermissions]);

  // Request microphone permission
  const handleMicrophoneAction = async () => {
    if (permissions.microphone === 'denied') {
      // Try to open system settings
      try {
        await openPrivacySettings('microphone');
      } catch {
        toast.error('Could not open System Settings', {
          description: 'Please enable microphone access in System Settings → Privacy & Security → Microphone.',
        });
      }
      return;
    }

    setIsPending(true);
    try {
      console.log('[PermissionsStep] Triggering microphone permission...');
      const granted = await invoke<boolean>('trigger_microphone_permission');
      console.log('[PermissionsStep] Microphone permission result:', granted);

      if (granted) {
        setPermissionStatus('microphone', 'authorized');
      } else {
        // Permission was denied or dialog was dismissed
        setPermissionStatus('microphone', 'denied');
      }
    } catch (err) {
      console.error('[PermissionsStep] Failed to request microphone permission:', err);
      setPermissionStatus('microphone', 'denied');
    } finally {
      setIsPending(false);
    }
  };

  // Request system audio permission
  const handleSystemAudioAction = async () => {
    if (permissions.systemAudio === 'denied') {
      // Try to open system settings
      try {
        await openPrivacySettings('systemAudio');
      } catch {
        toast.error('Could not open System Settings', {
          description: 'Please enable Audio Capture in System Settings → Privacy & Security → Audio Capture.',
        });
      }
      return;
    }

    setIsPending(true);
    try {
      console.log('[PermissionsStep] Triggering Audio Capture permission...');
      // Backend creates the Core Audio tap, plays a brief tone, and probes
      // whether the tap actually heard it (specs/0061 W3) instead of assuming
      // grant from tap construction alone.
      const probe = await invoke<ProbeResult>('trigger_system_audio_permission_command');
      console.log('[PermissionsStep] System audio probe result:', probe);

      if (probe.state === 'granted') {
        setPermissionStatus('systemAudio', 'authorized');
        console.log('[PermissionsStep] Audio Capture permission verified - real audio heard');
      } else if (probe.state === 'silent') {
        // The tap opened but only heard silence — honest "not yet", not a
        // denial. No toast: the row itself carries the copy.
        setPermissionStatus('systemAudio', 'silent');
        console.log('[PermissionsStep] Audio Capture probe silent - no audio heard yet');
      } else {
        // Tap construction itself failed.
        setPermissionStatus('systemAudio', 'denied');
        console.log('[PermissionsStep] Audio Capture probe failed:', probe.message);
      }
    } catch (err) {
      console.error('[PermissionsStep] Failed to request system audio permission:', err);
      setPermissionStatus('systemAudio', 'denied');
    } finally {
      setIsPending(false);
    }
  };

  const handleFinish = () => {
    goNext();
  };

  const handleSkip = () => {
    setPermissionsSkipped(true);
    goNext();
  };

  const allPermissionsGranted =
    permissions.microphone === 'authorized' &&
    permissions.systemAudio === 'authorized';

  return (
    <OnboardingContainer
      title="Grant permissions"
      description="Nixon needs access to your microphone and system audio to record meetings"
      step={4}
      hideProgress={true}
      showNavigation={allPermissionsGranted}
      canGoNext={allPermissionsGranted}
    >
      <div className="max-w-lg mx-auto space-y-6">
        {/* Permission Rows */}
        <div className="space-y-4">
          {/* Microphone */}
          <PermissionRow
            title="Microphone"
            description="Required to capture your voice during meetings"
            status={permissions.microphone}
            isPending={isPending}
            onAction={handleMicrophoneAction}
          />

          {/* System Audio */}
          <PermissionRow
            title="System Audio"
            description="Click Enable to grant Audio Capture permission"
            status={permissions.systemAudio}
            isPending={isPending}
            onAction={handleSystemAudioAction}
          />
        </div>

        {/* Action Buttons */}
        <div className="flex flex-col gap-3 pt-4">
          <Button variant="brand" size="lg" className="w-full" onClick={handleFinish} disabled={!allPermissionsGranted}>
            Continue
          </Button>

          <Button variant="ghost" onClick={handleSkip} className="text-muted-foreground">
            I&apos;ll do this later
          </Button>

          {!allPermissionsGranted && (
            <p className="text-xs text-center text-muted-foreground">
              Recording won&apos;t work without permissions. You can grant them later in settings.
            </p>
          )}
        </div>
      </div>
    </OnboardingContainer>
  );
}
