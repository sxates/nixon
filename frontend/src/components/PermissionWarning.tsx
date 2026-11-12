import React from 'react';
import { openPrivacySettings } from '@/lib/system-settings';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { PermissionRow } from '@/components/ui/permission-row';
import { Spinner } from '@/components/ui/spinner';
import { useIsLinux } from '@/hooks/usePlatform';

interface PermissionWarningProps {
  hasMicrophone: boolean;
  hasSystemAudio: boolean;
  onRecheck: () => void;
  isRechecking?: boolean;
}

/**
 * The record page's "you can't record yet" notice: one amber `Callout` with Recheck on its
 * title line, and a `PermissionRow` per permission, each with its own Open-settings key
 * (outline — the card has no primary; docs/design-system.md §Component catalog).
 */
export function PermissionWarning({
  hasMicrophone,
  hasSystemAudio,
  onRecheck,
  isRechecking = false
}: PermissionWarningProps) {
  const isLinux = useIsLinux();

  // Don't show on Linux - permission handling is not needed
  if (isLinux) {
    return null;
  }

  // Don't show if both permissions are granted
  if (hasMicrophone && hasSystemAudio) {
    return null;
  }

  const isMacOS = navigator.userAgent.includes('Mac');

  const openMicrophoneSettings = async () => {
    if (isMacOS) {
      try {
        await openPrivacySettings('microphone');
      } catch (error) {
        console.error('Failed to open microphone settings:', error);
      }
    }
  };

  const openAudioCaptureSettings = async () => {
    if (isMacOS) {
      try {
        await openPrivacySettings('systemAudio');
      } catch (error) {
        console.error('Failed to open audio capture settings:', error);
      }
    }
  };

  return (
    <div className="mb-4 w-full max-w-xl">
      <Callout
        tone="warn"
        title="Permissions required"
        actionAlign="start"
        action={
          <Button variant="outline" size="sm" onClick={onRecheck} disabled={isRechecking}>
            {isRechecking && <Spinner size="sm" />}
            Recheck
          </Button>
        }
      >
        {/* Each permission: the row (status + its own key) and, under it at full width,
            what to check. In the row's description the checklist was squeezed into a
            narrow column beside the status and the key. */}
        <div className="mt-1 divide-y divide-rule">
          <div>
            <PermissionRow
              title="Microphone"
              state={hasMicrophone ? 'granted' : 'denied'}
              description={
                hasMicrophone
                  ? undefined
                  : 'Nixon needs access to your microphone to record meetings. No microphone devices were detected.'
              }
              action={
                isMacOS && !hasMicrophone ? (
                  <Button variant="outline" size="sm" onClick={openMicrophoneSettings}>
                    Open Microphone Settings
                  </Button>
                ) : undefined
              }
            />
            {!hasMicrophone && (
              <div className="pb-3 text-xs text-muted-foreground">
                <p className="font-medium text-foreground">Please check:</p>
                <ul className="ml-2 mt-0.5 list-inside list-disc space-y-0.5">
                  <li>Your microphone is connected and powered on</li>
                  <li>Microphone permission is granted in System Settings</li>
                  <li>No other app is exclusively using the microphone</li>
                </ul>
              </div>
            )}
          </div>
          <div>
            <PermissionRow
              title="System audio"
              state={hasSystemAudio ? 'granted' : 'denied'}
              description={
                hasSystemAudio
                  ? undefined
                  : hasMicrophone
                    ? 'System audio capture is not available. You can still record with your microphone, but computer audio won\'t be captured.'
                    : 'System audio capture is also not available.'
              }
              action={
                isMacOS && !hasSystemAudio ? (
                  <Button variant="outline" size="sm" onClick={openAudioCaptureSettings}>
                    Open Audio Capture Settings
                  </Button>
                ) : undefined
              }
            />
            {!hasSystemAudio && isMacOS && (
              <div className="pb-3 text-xs text-muted-foreground">
                <p className="font-medium text-foreground">To enable system audio on macOS:</p>
                <ul className="ml-2 mt-0.5 list-inside list-disc space-y-0.5">
                  <li>Allow Audio Capture for Nixon in System Settings → Privacy &amp; Security</li>
                  <li>Come back here and press Recheck</li>
                </ul>
                <p className="mt-1">
                  Nixon records computer audio directly — no extra audio driver or routing is needed.
                </p>
              </div>
            )}
          </div>
        </div>
      </Callout>
    </div>
  );
}
