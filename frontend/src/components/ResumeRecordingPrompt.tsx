'use client';

/**
 * Relaunch recovery prompt (specs/0037).
 *
 * A launch-level check (mounted once from app/layout.tsx, NOT the /record page): on mount
 * it asks the backend for any recording that was interrupted by a crash/force-quit
 * (`metadata.json.status == "recording"` never flipped to "completed"). If any are found,
 * it surfaces a modal — one at a time — offering to Resume (reopen the same meeting + folder
 * and continue capturing) or Discard. We NEVER silently reopen the mic (owner decision:
 * always prompt).
 */

import { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Radio } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { armResumeRecording } from '@/lib/resume-recording';

/** Matches the Rust `InterruptedRecording` (serde `rename_all = "camelCase"`). */
export interface InterruptedRecording {
  meetingId: string;
  folderPath: string;
  meetingName: string | null;
  startedAt: string;
  segmentCount: number;
}

/** "started 5 minutes ago" / "started at 3:14 PM" — best-effort, degrades to raw string. */
function formatStartedAt(startedAt: string): string {
  const started = new Date(startedAt);
  if (Number.isNaN(started.getTime())) return `started ${startedAt}`;

  const diffMs = Date.now() - started.getTime();
  const diffMin = Math.round(diffMs / 60000);
  if (diffMin < 1) return 'started just now';
  if (diffMin < 60) return `started ${diffMin} minute${diffMin === 1 ? '' : 's'} ago`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `started ${diffHr} hour${diffHr === 1 ? '' : 's'} ago`;

  return `started ${started.toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })}`;
}

export default function ResumeRecordingPrompt() {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  // FIFO queue of interrupted recordings; we show the head one at a time.
  const [queue, setQueue] = useState<InterruptedRecording[]>([]);
  const [busy, setBusy] = useState(false);

  // Scan once on mount. A recording that's already live means we relaunched into an
  // in-progress session (shouldn't happen, but guard anyway) — never prompt over it.
  useEffect(() => {
    let cancelled = false;
    const scan = async () => {
      try {
        const interrupted = await invoke<InterruptedRecording[]>('api_list_interrupted_recordings');
        if (!cancelled && Array.isArray(interrupted) && interrupted.length > 0) {
          setQueue(interrupted);
        }
      } catch (error) {
        // Non-fatal: recovery is best-effort. Don't toast on launch — just log.
        console.warn('Could not check for interrupted recordings:', error);
      }
    };
    void scan();
    return () => {
      cancelled = true;
    };
  }, []);

  const current = queue[0] ?? null;
  const open = !!current && !isRecording;

  const handleResume = useCallback(() => {
    if (!current) return;
    armResumeRecording({
      meetingId: current.meetingId,
      folderPath: current.folderPath,
      meetingName: current.meetingName,
    });
    setQueue([]); // close the prompt; the /record page takes over
    router.push('/record');
  }, [current, router]);

  const handleDiscard = useCallback(async () => {
    if (!current) return;
    setBusy(true);
    try {
      await invoke('api_discard_interrupted_recording', { meetingId: current.meetingId });
      setQueue((q) => q.slice(1)); // advance to the next interrupted recording, if any
    } catch (error) {
      console.error('Failed to discard interrupted recording:', error);
      toast.error('Could not discard the unfinished recording', {
        description: error instanceof Error ? error.message : 'Check the console for details.',
      });
    } finally {
      setBusy(false);
    }
  }, [current]);

  // Dismissing via Escape / outside-click / X = "not now": keep it for next launch rather
  // than discarding captured audio. Blocked while a discard is in flight.
  const handleOpenChange = useCallback(
    (next: boolean) => {
      if (busy) return;
      if (!next) setQueue([]);
    },
    [busy],
  );

  if (!current) return null;

  const title = current.meetingName?.trim() || 'Untitled';

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Radio className="h-5 w-5 text-record" />
            Unfinished recording
          </DialogTitle>
          <DialogDescription>
            {title} · {formatStartedAt(current.startedAt)}
          </DialogDescription>
        </DialogHeader>

        <p className="text-sm text-muted-foreground">
          This recording was interrupted before it finished saving. Resume to keep recording
          into the same meeting, or discard it.
        </p>

        <DialogFooter>
          <Button variant="outline" onClick={handleDiscard} disabled={busy}>
            Discard
          </Button>
          <Button
            onClick={handleResume}
            disabled={busy}
            className="bg-brand text-brand-foreground hover:bg-brand/90"
          >
            <Radio className="mr-2 h-4 w-4" />
            Resume recording
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
