'use client';

import { ConnectCalendarCard } from '@/components/Calendar/ConnectCalendarCard';

/**
 * Compact "connect your calendar" nudge (mirrors the Day Agenda affordance).
 *
 * specs/0066: this used to ask EventKit for access itself and, when the answer was no,
 * toast "grant access in System Settings" — sending you off to find a setting rather than
 * taking you to one. The card routes to Settings → Calendar now, where both sources are
 * offered and the permission prompt belongs, so there is nothing left here to handle.
 */
export function ConnectCalendarNudge() {
  return <ConnectCalendarCard title="Connect your calendar to see your full day" />;
}
