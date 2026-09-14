'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Calendar, Check, Loader2, Mic, Volume2 } from 'lucide-react';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import {
  type CalendarAccessStatus,
  getCalendarAccessStatus,
  requestCalendarAccess,
} from '@/lib/calendar';
import { usePermissionsModal } from '@/contexts/PermissionsModalContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

/** Tri-state per permission row, mirroring the OS authorization state. */
type RowState = 'unknown' | 'granted' | 'denied';

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
      const granted = await invoke<boolean>('trigger_system_audio_permission_command');
      setSystemAudio(granted ? 'granted' : 'denied');
    } catch (err) {
      console.warn('[PermissionsModal] trigger_system_audio_permission_command failed:', err);
    } finally {
      setPendingRow(null);
    }
  };

  // Recording is possible once mic + system audio are granted (calendar is
  // optional — it only enriches titles/participants).
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
      icon: <Calendar className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      state: calendar,
      primary: false,
      bordered: true,
      onAllow: handleCalendar,
    },
    {
      key: 'mic' as const,
      label: 'Microphone',
      sub: 'Capture your voice in the room',
      icon: <Mic className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      state: microphone,
      primary: true,
      bordered: true,
      onAllow: handleMicrophone,
    },
    {
      key: 'sys' as const,
      label: 'System audio recording',
      sub: 'Record other participants on the call',
      icon: <Volume2 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      state: systemAudio,
      primary: true,
      bordered: false,
      onAllow: handleSystemAudio,
    },
  ];

  return (
    <Dialog open={isOpen} onOpenChange={setOpen}>
      <DialogContent
        className="max-w-[440px] gap-0 overflow-hidden rounded-lg border-border bg-card p-0 shadow-[0_24px_60px_rgba(40,33,28,0.30)]"
      >
        {/* Header — record-dot badge + serif title + local-processing subtitle */}
        <div className="px-6 pb-2 pt-[22px]">
          <div className="flex items-center gap-2.5">
            <span
              className="flex h-[34px] w-[34px] flex-shrink-0 items-center justify-center rounded-[9px] bg-record/10"
              aria-hidden="true"
            >
              <span className="h-[11px] w-[11px] rounded-full bg-record" />
            </span>
            <DialogTitle className="font-display text-xl font-semibold tracking-tight text-foreground">
              Enable recording
            </DialogTitle>
          </div>
          <DialogDescription className="mt-2 text-[13.5px] leading-relaxed text-muted-foreground">
            Nixon needs a few permissions to record &amp; transcribe this meeting. Everything is
            processed locally on your Mac.
          </DialogDescription>
        </div>

        {/* Permission rows */}
        <div className="px-4 py-2">
          {rows.map((row) => {
            const isGranted = row.state === 'granted';
            const isDenied = row.state === 'denied';
            const isPending = pendingRow === row.key;
            return (
              <div
                key={row.key}
                className={cn(
                  'flex items-center gap-3 px-2 py-3',
                  row.bordered && 'border-b border-border'
                )}
              >
                <span className="flex h-[30px] w-[30px] flex-shrink-0 items-center justify-center rounded-lg bg-secondary">
                  {row.icon}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-[13.5px] font-semibold text-foreground">{row.label}</div>
                  <div className="text-xs text-muted-foreground">
                    {isDenied ? 'Denied — open System Settings to allow' : row.sub}
                  </div>
                </div>
                {isGranted ? (
                  <span className="flex items-center gap-1.5 text-[12.5px] font-semibold text-chart-4">
                    <Check className="h-3.5 w-3.5" aria-hidden="true" strokeWidth={2.5} />
                    Allowed
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={row.onAllow}
                    disabled={isPending}
                    className={cn(
                      'inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12.5px] font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60',
                      row.primary
                        ? 'bg-brand text-brand-foreground hover:bg-brand/90'
                        : 'border border-border bg-transparent text-foreground hover:bg-accent'
                    )}
                  >
                    {isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />}
                    {isDenied ? 'Open Settings' : 'Allow'}
                  </button>
                )}
              </div>
            );
          })}
        </div>

        {/* Footer — Not now + Start recording (gated on real mic+system grants) */}
        <div className="flex items-center gap-2.5 px-5 pb-5 pt-3.5">
          <DialogClose asChild>
            <button
              type="button"
              className="rounded-[10px] border border-border bg-transparent px-3.5 py-2.5 text-[13px] font-semibold text-foreground transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              Not now
            </button>
          </DialogClose>
          {recordReady ? (
            <button
              type="button"
              onClick={handleStartRecording}
              className="flex flex-1 items-center justify-center gap-2 rounded-[10px] bg-record px-3.5 py-2.5 text-[13px] font-semibold text-record-foreground transition-colors hover:bg-record/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className="h-2 w-2 rounded-full bg-current" aria-hidden="true" />
              Start recording
            </button>
          ) : (
            <div
              className="flex flex-1 items-center justify-center rounded-[10px] bg-secondary px-3.5 py-2.5 text-center text-[13px] font-semibold text-muted-foreground"
              aria-disabled="true"
            >
              Allow microphone &amp; system audio to continue
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
