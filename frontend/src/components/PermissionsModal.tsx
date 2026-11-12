'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { PermissionRow, type PermissionState } from '@/components/ui/permission-row';
import { Spinner } from '@/components/ui/spinner';
import { StatusDot } from '@/components/ui/status-dot';
import {
  type CalendarAccessStatus,
  getCalendarAccessStatus,
  requestCalendarAccess,
} from '@/lib/calendar';
import { usePermissionsModal } from '@/contexts/PermissionsModalContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { SILENT_AUDIO_CAPTURE_COPY } from '@/components/onboarding/shared/PermissionRow';
import type { ProbeResult } from '@/types/onboarding';

/**
 * Per permission row state, mirroring the OS authorization state. `silent`
 * is Audio Capture-specific (specs/0061 W3): the tap opened but only heard
 * silence, so permission is probably not granted yet — honest, but distinct
 * from a hard OS denial.
 */
type RowState = 'unknown' | 'granted' | 'denied' | 'silent';

/**
 * "Enable recording" permissions modal (design source: specs/0057 mockup).
 * Informational/management surface opened from the "Recording permissions"
 * control in Settings › General (spec 0038 WS7.a) and from first-run
 * prompting — it does NOT gate the record-start flow.
 *
 * Each row reflects REAL OS permission state and triggers the REAL grant flow:
 *   - Calendar:     `api_get_calendar_access_status` / `api_request_calendar_access`
 *   - Microphone:   `trigger_microphone_permission`  (settings fallback: `open_system_settings`)
 *   - System audio: `trigger_system_audio_permission_command` (settings fallback too)
 *
 * Every Tauri call degrades gracefully on the bare dev binary (commands may be
 * unavailable) — failures resolve to a sensible state instead of crashing.
 */
export default function PermissionsModal() {
  const { isOpen, setOpen, closePermissionsModal } = usePermissionsModal();
  const { handleRecordingToggle } = useSidebar();

  const [calendar, setCalendar] = useState<RowState>('unknown');
  const [microphone, setMicrophone] = useState<RowState>('unknown');
  const [systemAudio, setSystemAudio] = useState<RowState>('unknown');

  // Which row is mid-request (disables its button + shows a spinner).
  const [pendingRow, setPendingRow] = useState<'cal' | 'mic' | 'sys' | null>(null);

  const calendarStatusToRow = (status: CalendarAccessStatus): RowState =>
    status === 'authorized' ? 'granted' : status === 'notDetermined' ? 'unknown' : 'denied';

  // Read the current calendar status whenever the modal opens. Mic/system audio
  // are notDetermined until the user explicitly requests them (the OS gives no
  // pre-grant read), so they start "unknown" and update after a grant attempt.
  const refreshCalendar = useCallback(async () => {
    const status = await getCalendarAccessStatus();
    setCalendar(calendarStatusToRow(status));
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    void refreshCalendar();
  }, [isOpen, refreshCalendar]);

  // --- Calendar -------------------------------------------------------------
  const handleCalendar = async () => {
    setPendingRow('cal');
    try {
      // requestCalendarAccess never throws; it triggers the OS prompt when
      // notDetermined and returns false on the bare dev binary.
      await requestCalendarAccess();
      await refreshCalendar();
    } finally {
      setPendingRow(null);
    }
  };

  // --- Microphone -----------------------------------------------------------
  const handleMicrophone = async () => {
    if (microphone === 'denied') {
      // Already denied → the OS won't re-prompt; deep-link to System Settings.
      try {
        await invoke('open_system_settings');
      } catch (err) {
        console.warn('[PermissionsModal] open_system_settings failed:', err);
      }
      return;
    }
    setPendingRow('mic');
    try {
      const granted = await invoke<boolean>('trigger_microphone_permission');
      setMicrophone(granted ? 'granted' : 'denied');
    } catch (err) {
      console.warn('[PermissionsModal] trigger_microphone_permission failed:', err);
      // Command unavailable (bare dev binary) — leave state unchanged.
    } finally {
      setPendingRow(null);
    }
  };

  // --- System audio ---------------------------------------------------------
  const handleSystemAudio = async () => {
    if (systemAudio === 'denied') {
      try {
        await invoke('open_system_settings');
      } catch (err) {
        console.warn('[PermissionsModal] open_system_settings failed:', err);
      }
      return;
    }
    setPendingRow('sys');
    try {
      // Probes the tap with a real (quiet, brief) tone instead of assuming
      // grant from tap construction alone (specs/0061 W3).
      const probe = await invoke<ProbeResult>('trigger_system_audio_permission_command');
      if (probe.state === 'granted') {
        setSystemAudio('granted');
      } else if (probe.state === 'silent') {
        setSystemAudio('silent');
      } else {
        setSystemAudio('denied');
      }
    } catch (err) {
      console.warn('[PermissionsModal] trigger_system_audio_permission_command failed:', err);
    } finally {
      setPendingRow(null);
    }
  };

  // Recording is possible once mic + system audio are granted (calendar is
  // optional — it only enriches titles/participants). `silent` counts as
  // NOT granted: the tap opened but never heard real audio, so recording
  // would silently produce no system audio.
  const recordReady = microphone === 'granted' && systemAudio === 'granted';

  const handleStartRecording = () => {
    if (!recordReady) return;
    closePermissionsModal();
    // Reuse the same entry point the New-recording button uses (autostart →
    // /record). This modal is informational, so it does not gate that flow.
    handleRecordingToggle();
  };

  const rows = [
    {
      key: 'cal' as const,
      label: 'Calendar',
      sub: 'Pull meeting titles & participants',
      state: calendar,
      onAllow: handleCalendar,
    },
    {
      key: 'mic' as const,
      label: 'Microphone',
      sub: 'Capture your voice in the room',
      state: microphone,
      onAllow: handleMicrophone,
    },
    {
      key: 'sys' as const,
      label: 'Audio capture',
      sub: 'Record other participants on the call',
      state: systemAudio,
      onAllow: handleSystemAudio,
    },
  ];

  /** Row state onto the shared PermissionStatus: not asked yet and silent both read "Not yet". */
  const toPermissionState = (state: RowState): PermissionState =>
    state === 'granted' ? 'granted' : state === 'denied' ? 'denied' : 'silent';

  return (
    <Dialog open={isOpen} onOpenChange={setOpen}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Enable recording</DialogTitle>
          <DialogDescription>
            Nixon needs a few permissions to record &amp; transcribe this meeting. Everything is
            processed locally on your Mac.
          </DialogDescription>
        </DialogHeader>

        {/* Permission rows — the shared PermissionRow, ruled like a SettingsGroup. */}
        <div className="divide-y divide-border">
          {rows.map((row) => {
            const isGranted = row.state === 'granted';
            const isDenied = row.state === 'denied';
            const isSilent = row.state === 'silent';
            const isPending = pendingRow === row.key;
            return (
              <PermissionRow
                key={row.key}
                title={row.label}
                description={
                  isDenied
                    ? 'Denied — open System Settings to allow'
                    : isSilent
                      ? SILENT_AUDIO_CAPTURE_COPY
                      : row.sub
                }
                state={toPermissionState(row.state)}
                action={
                  isGranted ? undefined : (
                    <Button variant="outline" size="sm" onClick={row.onAllow} disabled={isPending}>
                      {isPending ? <Spinner /> : null}
                      {isDenied ? 'Open Settings' : 'Allow'}
                    </Button>
                  )
                }
              />
            );
          })}
        </div>

        {/* Footer — Not now + Start recording (gated on real mic+system grants) */}
        <DialogFooter className="gap-2 sm:space-x-0">
          <DialogClose asChild>
            <Button variant="outline">Not now</Button>
          </DialogClose>
          {recordReady ? (
            // REC red is right here: this button starts a recording. `Button` has no
            // `record` variant yet, so the fill is spelled out (flagged for the primitive).
            <Button
              className="bg-record text-record-foreground hover:bg-record/90 sm:flex-1"
              onClick={handleStartRecording}
            >
              <StatusDot className="bg-current" />
              Start recording
            </Button>
          ) : (
            <div
              className="flex items-center justify-center rounded-md bg-secondary px-3.5 py-2 text-center text-sm font-semibold text-muted-foreground sm:flex-1"
              aria-disabled="true"
            >
              Allow microphone &amp; system audio to continue
            </div>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
