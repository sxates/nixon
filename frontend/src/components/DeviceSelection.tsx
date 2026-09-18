import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RefreshCw, Mic, Speaker } from 'lucide-react';
import { AudioBackendSelector } from './AudioBackendSelector';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Label } from '@/components/ui/label';

export interface AudioDevice {
  name: string;
  device_type: 'Input' | 'Output';
}

export interface SelectedDevices {
  micDevice: string | null;
  systemDevice: string | null;
}

interface DeviceSelectionProps {
  selectedDevices: SelectedDevices;
  onDeviceChange: (devices: SelectedDevices) => void;
  disabled?: boolean;
}

export function DeviceSelection({ selectedDevices, onDeviceChange, disabled = false }: DeviceSelectionProps) {
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  // Filter devices by type
  const inputDevices = devices.filter(device => device.device_type === 'Input');
  const outputDevices = devices.filter(device => device.device_type === 'Output');

  // Fetch available audio devices
  const fetchDevices = async () => {
    try {
      setError(null);
      const result = await invoke<AudioDevice[]>('get_audio_devices');
      setDevices(result);
      console.log('Fetched audio devices:', result);
    } catch (err) {
      console.error('Failed to fetch audio devices:', err);
      setError('Failed to load audio devices. Please check your system audio settings.');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  };

  // Load devices on component mount
  useEffect(() => {
    fetchDevices();
  }, []);

  // Handle device refresh
  const handleRefresh = async () => {
    setRefreshing(true);
    await fetchDevices();
  };

  // Handle microphone device selection
  const handleMicDeviceChange = (deviceName: string) => {
    const newDevices = {
      ...selectedDevices,
      micDevice: deviceName === 'default' ? null : deviceName
    };
    onDeviceChange(newDevices);
  };

  // Handle system audio device selection
  const handleSystemDeviceChange = (deviceName: string) => {
    const newDevices = {
      ...selectedDevices,
      systemDevice: deviceName === 'default' ? null : deviceName
    };
    onDeviceChange(newDevices);
  };

  if (loading) {
    return (
      <div className="p-4 space-y-4">
        <div className="animate-pulse">
          <div className="h-4 bg-muted rounded w-1/3 mb-4"></div>
          <div className="h-10 bg-muted rounded mb-3"></div>
          <div className="h-10 bg-muted rounded"></div>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium text-foreground">Audio Devices</h4>
        <div className="flex items-center space-x-2">
          <button
            onClick={handleRefresh}
            disabled={refreshing || disabled}
            className="h-8 w-8 p-0 inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors hover:bg-muted disabled:pointer-events-none disabled:opacity-50"
          >
            <RefreshCw className={`h-4 w-4 ${refreshing ? 'animate-spin' : ''}`} />
          </button>
        </div>
      </div>

      {error && (
        <div className="p-3 text-sm text-foreground bg-destructive/10 border border-destructive/30 rounded-md">
          {error}
        </div>
      )}

      <div className="space-y-3">
        {/* Microphone Selection */}
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Mic className="h-4 w-4 text-muted-foreground" />
            <Label htmlFor="mic-selection" className="text-sm font-medium text-foreground">
              Microphone
            </Label>
          </div>
          <Select
            value={selectedDevices.micDevice || 'default'}
            onValueChange={handleMicDeviceChange}
            disabled={disabled}
          >
            <SelectTrigger id="mic-selection" className="w-full">
              <SelectValue placeholder="Select Microphone" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="default">Default Microphone</SelectItem>
              {inputDevices.map((device) => (
                <SelectItem
                  key={device.name}
                  value={`${device.name} (${device.device_type.toLowerCase()})`}
                >
                  {device.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {inputDevices.length === 0 && (
            <p className="text-xs text-muted-foreground">No microphone devices found</p>
          )}
        </div>

        {/* System Audio Selection */}
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Speaker className="h-4 w-4 text-muted-foreground" />
            <Label htmlFor="system-selection" className="text-sm font-medium text-foreground">
              System Audio
            </Label>
          </div>

          <Select
            value={selectedDevices.systemDevice || 'default'}
            onValueChange={handleSystemDeviceChange}
            disabled={disabled}
          >
            <SelectTrigger id="system-selection" className="w-full">
              <SelectValue placeholder="Select System Audio" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="default">Default System Audio</SelectItem>
              {outputDevices.map((device) => (
                <SelectItem
                  key={device.name}
                  value={`${device.name} (${device.device_type.toLowerCase()})`}
                >
                  {device.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          {outputDevices.length === 0 && (
            <p className="text-xs text-muted-foreground">No system audio devices found</p>
          )}

          {/* Backend Selection - available on all platforms */}
          {!disabled && (
            <div className="pt-3 border-t border-border">
              <AudioBackendSelector disabled={disabled} />
            </div>
          )}
        </div>
      </div>

      {/* Info text */}
      <div className="text-xs text-muted-foreground space-y-1">
        <p>• <strong>Microphone:</strong> Records your voice and ambient sound</p>
        <p>• <strong>System Audio:</strong> Records computer audio (music, calls, etc.)</p>
      </div>
    </div>
  );
}
