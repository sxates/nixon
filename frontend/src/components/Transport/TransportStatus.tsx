'use client';

import React from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useRecordingLevel } from '@/hooks/useRecordingLevel';
import { useMicGate } from '@/hooks/useMicGate';
import { cn } from '@/lib/utils';
import { Reels } from './Reels';
import { TapeCounter } from './TapeCounter';
import { LevelLadder } from './LevelLadder';

export type TransportPhase = 'idle' | 'starting' | 'recording' | 'paused' | 'finalizing';

/**
 * Left zone of the rail: reels · title/state · counter · ladder (specs/0057 §3.1 state table).
 *
 * The second line is the state vocabulary the whole app now shares — "Nothing on the reel" /
 * "On the reel" / "On hold" / "Finishing the reel" — so the rail never disagrees with itself
 * the way the old bar, pill and header trio did.
 */
export function TransportStatus({ phase, elapsedSeconds }: { phase: TransportPhase; elapsedSeconds: number }) {
  const { currentMeeting, activeRecordingMeetingId } = useSidebar();
  const level = useRecordingLevel(phase === 'recording');
  // specs/0049 — while Zoom's own mute is on, the owner mic is gated in the backend; the
  // ladder must not imply we are still capturing the user's voice.
  const micMuted = useMicGate();
  const router = useRouter();
  const pathname = usePathname();
  // The rail is a way back to the live meeting from anywhere else. On /record there is
  // nowhere to go, and when nothing is on the reel there is no meeting to go to.
  const canNavigate = phase !== 'idle' && pathname !== '/record';
  // When the user has navigated away from the meeting being recorded, `currentMeeting` is a
  // different one — fall back to the live session's own title before the bare literal.
  // ('+ New Call' is the context's unnamed-session placeholder, not a title.)
  const { meetingTitle } = useTranscripts();
  const liveTitle = meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle : '';
  // Source order matters (specs/0063 item 5): a rename lands in TranscriptContext
  // synchronously, while SidebarProvider is updated a beat later by
  // `useRecordingTitleEdit`. Preferring `liveTitle` means the rail never shows the old
  // name, and the sidebar mirror keeps the meetings list honest.
  const sidebarTitle =
    activeRecordingMeetingId && currentMeeting?.id === activeRecordingMeetingId ? currentMeeting.title : '';
  const title = liveTitle || sidebarTitle || 'Recording';
  const line1 =
    phase === 'idle' ? 'Deck ready' : phase === 'starting' ? 'Starting' : phase === 'finalizing' ? 'Saving' : title;
  const line2 =
    phase === 'idle'
      ? 'Nothing on the reel'
      : phase === 'starting'
        ? 'Arming the reel'
        : phase === 'finalizing'
          ? 'Finishing the reel'
          : phase === 'paused'
            ? 'On hold'
            : micMuted
              ? 'Mic muted'
              : 'On the reel';
  const tone = phase === 'paused' ? 'amber' : phase === 'idle' || phase === 'starting' ? 'dim' : 'normal';
  return (
    <div className="flex min-w-0 flex-1 items-center gap-3.5 px-5">
      {/* Nothing is on the reel yet while the tap is arming, so the hubs stay still. */}
      <Reels state={phase === 'starting' ? 'idle' : phase} />
      {canNavigate ? (
        <button
          type="button"
          onClick={() => router.push('/record')}
          aria-label="Back to the recording"
          className={cn(
            'flex min-w-0 flex-col items-start rounded-[3px] text-left leading-tight',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-panel',
            'hover:[&>span:first-child]:text-brand',
          )}
        >
          <span className="max-w-full truncate text-xs font-semibold text-foreground transition-colors [transition-duration:120ms]">
            {line1}
          </span>
          <span className="u-section-label text-[9px]">{line2}</span>
        </button>
      ) : (
        <div className="flex min-w-0 flex-col leading-tight">
          <span className="truncate text-xs font-semibold text-foreground">{line1}</span>
          <span className="u-section-label text-[9px]">{line2}</span>
        </div>
      )}
      <TapeCounter seconds={elapsedSeconds} size="sm" tone={tone} />
      <LevelLadder level={level.rms} active={phase === 'recording' && !micMuted} />
    </div>
  );
}
