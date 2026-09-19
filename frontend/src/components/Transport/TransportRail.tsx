'use client';

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { cn } from '@/lib/utils';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { recordingService } from '@/services/recordingService';
import { requestFullRecordingStop } from '@/lib/recording-stop';
import { TransportKey } from './TransportKey';
import { TransportStatus, type TransportPhase } from './TransportStatus';

/**
 * specs/0057 decision 7 — THE recording control surface. Fixed to the bottom edge, from the
 * sidebar's right edge to the window edge, on every screen. Replaces GlobalRecordingBar,
 * the /record floating pill and the RecordingHeader button pair (one state vocabulary,
 * spec §3.1 table). The queue it used to host on its right moved to the sidebar in
 * specs/0064 W5 — the rail is the recording surface, the queue is app state.
 *
 * Stop deliberately goes through `requestFullRecordingStop`, not a raw `stop_recording`: the
 * complete stop is the command PLUS the post-stop save flow that only /record mounts.
 */
export function TransportRail() {
  const rs = useRecordingState();
  const { isCollapsed, handleRecordingToggle } = useSidebar();
  const router = useRouter();

  const finalizing = rs.isStopping || rs.isProcessing || rs.isSaving;
  // `starting` is evaluated after `finalizing` and before the isRecording branches: the tap
  // is arming (RecordingStatus.STARTING, useRecordingStart) and `isRecording` is still false,
  // so without this the rail would say "Deck ready" and offer REC again mid-start.
  const phase: TransportPhase = finalizing
    ? 'finalizing'
    : rs.status === 'starting'
      ? 'starting'
      : rs.isRecording
        ? rs.isPaused
          ? 'paused'
          : 'recording'
        : 'idle';
  const inFlight = phase !== 'idle';
  const elapsed = rs.activeDuration ?? rs.recordingDuration ?? 0;

  const onRec = useCallback(() => {
    if (phase === 'idle') handleRecordingToggle();
  }, [phase, handleRecordingToggle]);
  // The pause/resume command can fail (device gone, backend mid-stop). Swallowing it left HOLD
  // looking like it worked — surface it, and ignore a second click while the first is in flight.
  //
  // The guard is a GENERATION counter, not a boolean. It is bumped on every press AND on every
  // phase change, and a press's `finally` only clears the busy state if its own generation is
  // still current. That covers both failure modes: an invoke that never settles can't wedge
  // HOLD for the session (the phase change releases it), and the phase change that arrives
  // BEFORE the first invoke settles — `isPaused` flips on a backend event, which can beat the
  // command's own promise — can't let that first `finally` clear the busy state belonging to a
  // second, still-running press.
  const holdGeneration = useRef(0);
  const holdInFlight = useRef(false);
  const [holdBusy, setHoldBusy] = useState(false);
  const onHold = useCallback(async () => {
    if (phase !== 'recording' && phase !== 'paused') return;
    if (holdInFlight.current) return;
    const resuming = phase === 'paused';
    const generation = ++holdGeneration.current;
    holdInFlight.current = true;
    setHoldBusy(true);
    try {
      if (resuming) await recordingService.resumeRecording();
      else await recordingService.pauseRecording();
      // isPaused is updated by RecordingStateContext via backend events.
    } catch (error) {
      console.error('[TransportRail] Failed to pause/resume:', error);
      toast.error(resuming ? 'Could not resume recording' : 'Could not pause recording', {
        description: 'Please try again, or open the recording to use the full controls.',
      });
    } finally {
      // A superseded press (the phase moved on, or another press started) owns nothing here.
      if (holdGeneration.current === generation) {
        holdInFlight.current = false;
        setHoldBusy(false);
      }
    }
  }, [phase]);
  // Any phase transition is proof the recording moved on, so it releases the guard — and
  // supersedes whatever press was in flight.
  useEffect(() => {
    holdGeneration.current += 1;
    holdInFlight.current = false;
    setHoldBusy(false);
  }, [phase]);
  const onStop = useCallback(() => {
    if (phase === 'recording' || phase === 'paused') requestFullRecordingStop((p) => router.push(p));
  }, [phase, router]);

  return (
    <div
      role="region"
      aria-label={inFlight ? 'Recording in progress' : 'Transport'}
      className={cn(
        'fixed bottom-0 right-0 z-40 flex h-[var(--rail-h)] items-stretch border-t border-border',
        'bg-panel shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),0_-8px_16px_-12px_rgba(0,0,0,0.35)]',
        'transition-[left] duration-300',
        isCollapsed ? 'left-16' : 'left-64',
      )}
    >
      {/* Instruments first, keys last (specs/0064 W6) — the status zone takes the slack, so
          the keys sit against the right edge whatever the meeting is called. */}
      <TransportStatus phase={phase} elapsedSeconds={elapsed} />
      <div className="w-px self-stretch bg-border" />
      <div className="flex flex-none items-center gap-1.5 px-5">
        <TransportKey
          fn="rec"
          legend="REC"
          lit={inFlight && phase !== 'finalizing'}
          holding={phase === 'paused'}
          disabled={phase !== 'idle'}
          aria-label={phase === 'idle' ? 'Start recording' : 'Recording'}
          onClick={onRec}
        />
        <TransportKey
          fn="hold"
          legend="HOLD"
          lit={phase === 'paused'}
          disabled={holdBusy || phase === 'idle' || phase === 'starting' || phase === 'finalizing'}
          aria-label={phase === 'paused' ? 'Resume recording' : 'Pause recording'}
          onClick={onHold}
        />
        <TransportKey
          fn="stop"
          legend="STOP"
          disabled={phase === 'idle' || phase === 'starting' || phase === 'finalizing'}
          aria-label="Stop recording"
          onClick={onStop}
        />
      </div>
    </div>
  );
}

export default TransportRail;
