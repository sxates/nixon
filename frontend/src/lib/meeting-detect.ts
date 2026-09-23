/**
 * The detected-call prompt's wording and wire shape (specs/0074 W5).
 *
 * The backend emits one event when a call starts and one when it ends. Only Zoom is
 * detected today; W6 adds Teams and Google Meet and names the app in an optional
 * `platform` field. An event without one is Zoom, so the prompt reads the same as before
 * until the backend starts sending it.
 */

/** Payload of the meeting detected / ended events. */
export interface MeetingDetectEvent {
  timestamp_ms?: number;
  kind?: 'detected' | 'ended';
  /** `zoom` | `teams` | `meet`. Absent means Zoom (the only detector before W6). */
  platform?: string | null;
}

const PLATFORM_NAMES: Record<string, string> = {
  zoom: 'Zoom',
  teams: 'Teams',
  meet: 'Google Meet',
};

/** "Zoom call detected", "Teams call detected", … — "Call detected" for a name we don't know. */
export function detectedTitle(platform?: string | null): string {
  const key = (platform ?? 'zoom').trim().toLowerCase();
  const name = PLATFORM_NAMES[key];
  return name ? `${name} call detected` : 'Call detected';
}

export const DETECTED_BODY = 'Record this meeting?';

/** Added to the prompt when the OS banner could not be sent (permission off, delivery error). */
export const NOTIFICATIONS_OFF_COPY = 'macOS notifications are off for Nixon.';

/** Where the Enable action goes: the General tab holds the notification permission row. */
export const NOTIFICATION_SETTINGS_ROUTE = '/settings?tab=general';

/** Banner id for one detection: `nixon-detected-<backend timestamp>`. */
export function detectedBannerId(event?: MeetingDetectEvent | null): string {
  return `nixon-detected-${event?.timestamp_ms ?? Date.now()}`;
}
