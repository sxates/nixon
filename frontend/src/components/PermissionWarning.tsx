import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { Spinner } from '@/components/ui/spinner';
import { useIsLinux } from '@/hooks/usePlatform';

interface PermissionWarningProps {
  hasMicrophone: boolean;
  hasSystemAudio: boolean;
  onRecheck: () => void;
  isRechecking?: boolean;
}

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
        await invoke('open_system_settings', { preferencePane: 'Privacy_Microphone' });
      } catch (error) {
        console.error('Failed to open microphone settings:', error);
      }
    }
  };

  const openAudioCaptureSettings = async () => {
    if (isMacOS) {
      try {
        await invoke('open_system_settings', { preferencePane: 'Privacy_ScreenCapture' });
      } catch (error) {
        console.error('Failed to open audio capture settings:', error);
      }
    }
  };

  const title =
    !hasMicrophone && !hasSystemAudio
      ? 'Permissions Required'
      : !hasMicrophone
        ? 'Microphone Permission Required'
        : 'System Audio Permission Required';

  return (
    <div className="mb-4 max-w-md">
      <Callout tone="warn">
        <div className="font-semibold text-foreground">{title}</div>
        <div className="mt-3 flex flex-wrap gap-2">
          {isMacOS && !hasMicrophone && (
            <Button variant="brand" size="sm" onClick={openMicrophoneSettings}>
              Open Microphone Settings
            </Button>
          )}
          {isMacOS && !hasSystemAudio && (
            <Button variant="brand" size="sm" onClick={openAudioCaptureSettings}>
              Open Audio Capture Settings
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={onRecheck} disabled={isRechecking}>
            {isRechecking && <Spinner size="sm" />}
            Recheck
          </Button>
        </div>
        <div className="mt-3 space-y-3 text-sm text-foreground">
          {/* Microphone Warning */}
          {!hasMicrophone && (
            <div>
              <p className="mb-2">
                Nixon needs access to your microphone to record meetings. No microphone devices were detected.
              </p>
              <p className="font-medium">Please check:</p>
              <ul className="ml-2 mt-1 list-inside list-disc space-y-1">
                <li>Your microphone is connected and powered on</li>
                <li>Microphone permission is granted in System Settings</li>
                <li>No other app is exclusively using the microphone</li>
              </ul>
            </div>
          )}

          {/* System Audio Warning */}
          {!hasSystemAudio && (
            <div>
              <p className="mb-2">
                {hasMicrophone
                  ? 'System audio capture is not available. You can still record with your microphone, but computer audio won\'t be captured.'
                  : 'System audio capture is also not available.'}
              </p>
              {isMacOS && (
                <>
                  <p className="font-medium">To enable system audio on macOS:</p>
                  <ul className="ml-2 mt-1 list-inside list-disc space-y-1">
                    <li>Allow Audio Capture for Nixon in System Settings → Privacy &amp; Security</li>
                    <li>Come back here and press Recheck</li>
                  </ul>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Nixon records computer audio directly — no extra audio driver or routing is needed.
                  </p>
                </>
              )}
            </div>
          )}
        </div>
      </Callout>
    </div>
  );
}
