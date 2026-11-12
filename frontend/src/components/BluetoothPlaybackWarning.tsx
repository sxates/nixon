"use client";
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { X } from '@/components/ui/icons';

interface AudioOutputInfo {
  device_name: string;
  is_bluetooth: boolean;
  sample_rate: number | null;
  device_type: string;
}

interface BluetoothPlaybackWarningProps {
  /** Check interval in milliseconds (default: 5000ms / 5 seconds) */
  checkInterval?: number;
  /** Whether to show the warning (default: true for meeting playback pages) */
  enabled?: boolean;
}

export function BluetoothPlaybackWarning({
  checkInterval = 5000,
  enabled = true
}: BluetoothPlaybackWarningProps) {
  const [isBluetoothActive, setIsBluetoothActive] = useState(false);
  const [deviceName, setDeviceName] = useState<string>('');
  const [isDismissed, setIsDismissed] = useState(false);

  useEffect(() => {
    if (!enabled) return;

    const checkAudioOutput = async () => {
      try {
        const outputInfo = await invoke<AudioOutputInfo>('get_active_audio_output');

        if (outputInfo.is_bluetooth) {
          setIsBluetoothActive(true);
          setDeviceName(outputInfo.device_name);
        } else {
          setIsBluetoothActive(false);
          setIsDismissed(false); // Reset dismissal when switching to non-BT device
        }
      } catch (error) {
        console.error('Failed to check audio output device:', error);
        // Fail silently - don't show warning if we can't detect device
        setIsBluetoothActive(false);
      }
    };

    // Check immediately on mount
    checkAudioOutput();

    // Set up periodic checks
    const interval = setInterval(checkAudioOutput, checkInterval);

    return () => clearInterval(interval);
  }, [checkInterval, enabled]);

  // Don't show warning if Bluetooth not active, already dismissed, or not enabled
  if (!enabled || !isBluetoothActive || isDismissed) {
    return null;
  }

  return (
    <Callout
      tone="warn"
      className="mb-4"
      role="alert"
      aria-live="polite"
      title="Bluetooth Playback Detected"
      action={
        <IconButton
          variant="ghost"
          label="Dismiss warning"
          onClick={() => setIsDismissed(true)}
          icon={<DeckIcon icon={X} size={14} />}
        />
      }
    >
      You&apos;re using <strong>{deviceName}</strong> for playback.
      Recordings may sound distorted or sped up through Bluetooth devices.
      For accurate review, please use <strong>computer speakers</strong> or{' '}
      <strong>wired headphones</strong>.
    </Callout>
  );
}
