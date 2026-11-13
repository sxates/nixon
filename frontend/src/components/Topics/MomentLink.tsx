'use client';

/**
 * A small time link ("12:34") that opens a meeting at that moment (specs/0089). Shared by
 * the timeline's bullets, decisions and revisions and by the suggestion strip.
 */

import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { openMeetingMoment } from '@/lib/meetingLink';
import { cn } from '@/lib/utils';

/** Seconds -> "m:ss", or "h:mm:ss" past the hour. */
export function formatMoment(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

export interface MomentLinkProps {
  meetingId: string;
  t: number;
  className?: string;
}

export function MomentLink({ meetingId, t, className }: MomentLinkProps) {
  const router = useRouter();
  return (
    <Button
      type="button"
      variant="link"
      title="Open the meeting here"
      onClick={() => void openMeetingMoment(router, meetingId, t)}
      className={cn(
        'h-auto p-0 align-baseline font-mono text-caption font-normal text-muted-foreground hover:text-brand',
        className,
      )}
    >
      {formatMoment(t)}
    </Button>
  );
}

/** The meeting title as a link to the meeting (no moment). */
export function MeetingLink({ meetingId, title }: { meetingId: string; title: string }) {
  const router = useRouter();
  return (
    <Button
      type="button"
      variant="link"
      onClick={() => void openMeetingMoment(router, meetingId, null)}
      className="h-auto min-w-0 truncate p-0 text-meta font-semibold text-foreground hover:text-brand"
    >
      {title}
    </Button>
  );
}
