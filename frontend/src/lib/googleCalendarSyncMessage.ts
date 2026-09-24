/**
 * What to tell the user after a Google Calendar sync (specs/0074 W2, 0075 W1b).
 *
 * The old "Sync now" toasted a green "Google Calendar synced" whenever the invoke
 * resolved — including when nothing ran because another pass held the lock or the
 * grant had lapsed. Here only `kind: 'synced'` can ever produce a success message;
 * every other outcome names what actually happened.
 */

import { type GoogleCalendarSyncOutcome, syncChangeCount } from '@/lib/googleCalendar';

export type SyncMessageTone = 'success' | 'warning' | 'error' | 'info';

export interface SyncMessage {
  tone: SyncMessageTone;
  title: string;
  description?: string;
  /** The grant lapsed — the toast should offer a Reconnect action. */
  reconnect?: boolean;
}

function changesPhrase(n: number): string {
  if (n === 0) return 'no changes';
  return n === 1 ? '1 change' : `${n} changes`;
}

export function describeSyncOutcome(outcome: GoogleCalendarSyncOutcome): SyncMessage {
  switch (outcome.kind) {
    case 'synced': {
      const failed = outcome.calendars.filter((c) => c.error);
      if (failed.length === 1) {
        return {
          tone: 'warning',
          title: `Synced, but ${failed[0].summary} failed`,
          description: failed[0].error ?? undefined,
        };
      }
      if (failed.length > 1) {
        return {
          tone: 'warning',
          title: `Synced, but ${failed.length} calendars failed`,
          description: failed.map((c) => `${c.summary}: ${c.error}`).join('\n'),
        };
      }
      return {
        tone: 'success',
        title: `Google Calendar synced — ${changesPhrase(syncChangeCount(outcome))}`,
      };
    }
    case 'authRequired':
      return {
        tone: 'error',
        title: 'Google needs you to reconnect',
        description:
          'Your Google session expired or was revoked. Nothing syncs until you reconnect.',
        reconnect: true,
      };
    case 'alreadyRunning':
      return {
        tone: 'info',
        title: 'A sync is still running',
        description: "The last one hasn't finished yet. Try again in a minute.",
      };
    case 'notConnected':
      return { tone: 'info', title: "Nothing synced — Google Calendar isn't connected" };
    case 'noCalendarsSelected':
      return {
        tone: 'info',
        title: 'Nothing synced — no calendars are selected',
        description: 'Tick a calendar under "Calendars to sync" to sync it.',
      };
    case 'suppressed':
      return {
        tone: 'info',
        title: 'Nothing synced — demo data is loaded',
        description: 'Google Calendar sync is paused while the demo meetings are in use.',
      };
    case 'notConfigured':
      return { tone: 'info', title: "Nothing synced — Google Calendar isn't set up in this build" };
  }
}
