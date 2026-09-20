'use client';

/**
 * The "connect a calendar" card, shared by Today's nudge and the Upcoming section
 * (specs/0066 W5).
 *
 * It exists as one component because the copy was wrong in both places in the same way:
 * each said "Nixon reads your macOS Calendar", and each offered a single Connect button
 * that asks EventKit for permission. Google Calendar has been a first-class source since
 * specs/0032 — and is the likelier one for most people — but it was reachable only by
 * knowing to go to Settings → General → Calendar. Two copies of that sentence is how it
 * drifted twice; one copy is how it stays true.
 *
 * The primary action stays EventKit, because that is the one that can be granted from
 * here. Google's OAuth flow lives in Settings and stays there, so this routes to it.
 */

import { useRouter } from 'next/navigation';
import { Calendar } from 'lucide-react';
import { Button } from '@/components/ui/button';

export function ConnectCalendarCard({
  title,
  connecting,
  onConnect,
  onDismiss,
}: {
  title: string;
  connecting: boolean;
  onConnect: () => void;
  /** Omitted where the card has no dismiss affordance (Today's nudge). */
  onDismiss?: () => void;
}) {
  const router = useRouter();
  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
      <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-muted">
        <Calendar className="h-4 w-4 text-muted-foreground" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-foreground">{title}</div>
        <div className="text-xs text-muted-foreground">
          Nixon reads your macOS or Google Calendar on-device — nothing leaves your machine.
        </div>
      </div>
      <Button
        variant="brand"
        size="sm"
        className="flex-shrink-0"
        onClick={onConnect}
        disabled={connecting}
      >
        {connecting ? 'Connecting…' : 'Connect'}
      </Button>
      <button
        type="button"
        onClick={() => router.push('/settings?tab=general')}
        className="flex-shrink-0 whitespace-nowrap text-xs text-muted-foreground hover:text-foreground"
      >
        Use Google
      </button>
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          className="flex-shrink-0 text-xs text-muted-foreground hover:text-foreground"
        >
          Dismiss
        </button>
      )}
    </div>
  );
}
