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
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';

/** Settings → General, scrolled to the Calendar section (`CALENDAR_SECTION_ID`). */
export const CALENDAR_SETTINGS_ROUTE = '/settings?tab=general#calendar';

export function ConnectCalendarCard({
  title,
  onDismiss,
}: {
  title: string;
  /** Optional: when absent, the card renders with no dismiss control. Both current
   *  callers (Today's nudge, the Upcoming section's card) pass one; the omitted case is
   *  exercised by this component's own tests, not by production code. */
  onDismiss?: () => void;
}) {
  const router = useRouter();
  // The same Callout as Google's reconnect row: primary action first, then the quiet one.
  return (
    <Callout
      tone="info"
      title={title}
      action={
        <>
          <Button variant="brand" size="sm" onClick={() => router.push(CALENDAR_SETTINGS_ROUTE)}>
            Connect
          </Button>
          {onDismiss && (
            <Button variant="ghost" size="sm" onClick={onDismiss}>
              Dismiss
            </Button>
          )}
        </>
      }
    >
      Connect your Mac&apos;s calendar or Google Calendar in Settings — either is read
      on-device, and nothing leaves your machine.
    </Callout>
  );
}
