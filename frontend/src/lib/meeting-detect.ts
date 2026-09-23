/**
 * The detected-call prompt's wording and wire shape (specs/0074 W5 + W6).
 *
 * The backend emits `meeting-detected` when a Zoom, Teams or Google Meet call starts and
 * `meeting-ended` when it ends, naming the app in `platform`. An event without one is Zoom
 * (the only detector before W6).
 */

/** Backend event: a call started (not recording, detection on). */
export const MEETING_DETECTED_EVENT = 'meeting-detected';

/** Backend event: the detected call ended. */
export const MEETING_ENDED_EVENT = 'meeting-ended';

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

/**
 * Does this call ending stop a recording? Only Zoom's (or an event that names no platform,
 * which is Zoom). Zoom tears its meeting helpers down on leave; Teams and browsers may let go
 * of the microphone on mute, a device switch or a breakout, and stopping a recording on mute
 * is far worse than not stopping it at hang-up. For them, the end only takes the prompt down.
 */
export function endStopsRecording(platform?: string | null): boolean {
  return (platform ?? 'zoom').trim().toLowerCase() === 'zoom';
}
