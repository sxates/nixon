import React from 'react';
import { AlertTriangle, Mic, Speaker, RefreshCw } from 'lucide-react';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { invoke } from '@tauri-apps/api/core';
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

  return (
    <div className="max-w-md mb-4 space-y-3">
      {/* Combined Permission Warning - Show when either permission is missing */}
      {(!hasMicrophone || !hasSystemAudio) && (
        <Alert className="border-brand/30 bg-brand/10 [&>svg]:text-brand">
          <AlertTriangle className="h-5 w-5 text-brand" />
          <AlertTitle className="text-brand font-semibold">
            <div className="flex items-center gap-2">
              {!hasMicrophone && <Mic className="h-4 w-4" />}
              {!hasSystemAudio && <Speaker className="h-4 w-4" />}
              {!hasMicrophone && !hasSystemAudio ? 'Permissions Required' : !hasMicrophone ? 'Microphone Permission Required' : 'System Audio Permission Required'}
            </div>
          </AlertTitle>
          {/* Action Buttons */}
          <div className="mt-4 flex flex-wrap gap-2">
            {isMacOS && !hasMicrophone && (
              <button
                onClick={openMicrophoneSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-brand-foreground bg-brand hover:bg-brand/90 rounded-md transition-colors"
              >
                <Mic className="h-4 w-4" />
                Open Microphone Settings
              </button>
            )}
            {isMacOS && !hasSystemAudio && (
              <button
                onClick={openAudioCaptureSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-brand-foreground bg-brand hover:bg-brand/90 rounded-md transition-colors"
              >
                <Speaker className="h-4 w-4" />
                Open Audio Capture Settings
              </button>
            )}
            <button
              onClick={onRecheck}
              disabled={isRechecking}
              className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-foreground bg-muted hover:bg-accent rounded-md transition-colors disabled:opacity-50"
            >
              <RefreshCw className={`h-4 w-4 ${isRechecking ? 'animate-spin' : ''}`} />
              Recheck
            </button>
          </div>
          <AlertDescription className="text-foreground mt-2">
            {/* Microphone Warning */}
            {!hasMicrophone && (
              <>
                <p className="mb-3">
                  Nixon needs access to your microphone to record meetings. No microphone devices were detected.
                </p>
                <div className="space-y-2 text-sm mb-4">
                  <p className="font-medium">Please check:</p>
                  <ul className="list-disc list-inside ml-2 space-y-1">
                    <li>Your microphone is connected and powered on</li>
                    <li>Microphone permission is granted in System Settings</li>
                    <li>No other app is exclusively using the microphone</li>
                  </ul>
                </div>
              </>
            )}

            {/* System Audio Warning */}
            {!hasSystemAudio && (
              <>
                <p className="mb-3">
                  {hasMicrophone
                    ? 'System audio capture is not available. You can still record with your microphone, but computer audio won\'t be captured.'
                    : 'System audio capture is also not available.'}
                </p>
                {isMacOS && (
                  <div className="space-y-2 text-sm mb-4">
                    <p className="font-medium">To enable system audio on macOS:</p>
                    <ul className="list-disc list-inside ml-2 space-y-1">
                      <li>Install a virtual audio device (e.g., BlackHole 2ch)</li>
                      <li>Grant Audio Capture permission to Nixon</li>
                      <li>Configure your audio routing in Audio MIDI Setup</li>
                    </ul>
                  </div>
                )}
              </>
            )}


          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}
