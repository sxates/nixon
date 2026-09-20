'use client';

/**
 * The "connect a calendar" card, shared by Today's nudge and the Upcoming section
 * (specs/0066).
 *
 * One button, and it goes to Settings. The first version of this card had two — a Connect
 * that asked EventKit directly and a separate "Use Google" — which was wrong twice over
 * (owner feedback): asking EventKit from here can only end in a toast telling you to go
 * somewhere else when the answer is no, and splitting the two sources made the card argue
 * about a choice that Settings already presents properly. Nixon supports one calendar
 * source at a time, chosen in Settings → General → Calendar; this card's whole job is to
 * say a calendar would help and put you in front of that choice.
 *
 * It exists as one component because the copy was wrong in both places in the same way:
 * each said "Nixon reads your macOS Calendar", years after Google became a first-class
 * source. Two copies of a sentence is how it drifted twice; one copy is how it stays true.
 */

import { useRouter } from 'next/navigation';
import { Calendar } from 'lucide-react';
import { Button } from '@/components/ui/button';

/** Settings → General, scrolled to the Calendar section (`CALENDAR_SECTION_ID`). */
export const CALENDAR_SETTINGS_ROUTE = '/settings?tab=general#calendar';

export function ConnectCalendarCard({
  title,
  onDismiss,
}: {
  title: string;
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
          Connect your Mac&apos;s calendar or Google Calendar in Settings — either is read
          on-device, and nothing leaves your machine.
        </div>
      </div>
      <Button
        variant="brand"
        size="sm"
        className="flex-shrink-0"
        onClick={() => router.push(CALENDAR_SETTINGS_ROUTE)}
      >
        Connect
      </Button>
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
