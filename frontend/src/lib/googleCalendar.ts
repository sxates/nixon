/**
 * Google Calendar provider IPC helpers (specs/0032 task 6).
 *
 * Thin never-throw wrappers over the `api_google_calendar_*` Tauri commands,
 * mirroring the `lib/calendar.ts` house style: every helper swallows IPC errors
 * and returns a safe default, so the Settings card renders fine even while the
 * backend commands don't exist yet (parallel build) or fail at runtime.
 *
 * Backend contract (pinned, specs/0032 "Tauri IPC"; backend serializes camelCase):
 *   - `api_google_calendar_status`                → GoogleCalendarStatus
 *   - `api_google_calendar_connect`               → { email } — long-running (the
 *     user completes browser consent; the command times out after 5 minutes) and
 *     rejects with a user-friendly string on failure/cancel.
 *   - `api_google_calendar_disconnect`            → void
 *   - `api_google_calendar_set_calendar_selected` → void, args `{ calendarId, selected }`
 *     (Tauri matches invoke arg keys in camelCase — `calendarId`, never `calendar_id`)
 *   - `api_google_calendar_sync_now`              → GoogleCalendarSyncOutcome (specs/0074 W1)
 *   - Rust→frontend event `google-calendar-auth-required` when the stored token is
 *     revoked/expired and the user must reconnect.
 *   - Rust→frontend event `google-calendar-synced` `{ changed }` after a pass that
 *     wrote or removed rows.
 */

import { invoke } from '@tauri-apps/api/core';

/** Rust→frontend event: the stored Google token was revoked/expired — reconnect. */
export const GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT = 'google-calendar-auth-required';

/** Rust→frontend event: a sync pass changed the cache. Payload `{ changed: number }`. */
export const GOOGLE_CALENDAR_SYNCED_EVENT = 'google-calendar-synced';

/** How one calendar was synced (Rust `SyncMode`). */
export type GoogleCalendarSyncMode =
  | 'incremental'
  | 'full'
  | 'fullAfterGone'
  | 'fullAfterSeriesChange';

/** One calendar's result within a `synced` outcome (Rust `CalendarSyncReport`). */
export interface GoogleCalendarSyncReport {
  calendarId: string;
  summary: string;
  isPrimary: boolean;
  mode: GoogleCalendarSyncMode;
  fetched: number;
  upserted: number;
  deleted: number;
  durationMs: number;
  /** This calendar failed (the cache keeps serving); the pass still ran. */
  error: string | null;
}

/**
 * What `api_google_calendar_sync_now` (and an enabling selection change) reports
 * (Rust `SyncOutcome`, specs/0074 W1). Only `synced` means a pass ran; an `Err`
 * (pass couldn't start / timed out) still rejects the invoke with a message.
 */
export type GoogleCalendarSyncOutcome =
  | { kind: 'synced'; calendars: GoogleCalendarSyncReport[]; durationMs: number }
  | { kind: 'alreadyRunning' }
  | { kind: 'authRequired' }
  | { kind: 'notConnected' }
  | { kind: 'notConfigured' }
  | { kind: 'suppressed' }
  | { kind: 'noCalendarsSelected' };

/** One calendar on the connected Google account, with its per-calendar sync toggle. */
export interface GoogleCalendarListEntry {
  id: string;
  summary: string;
  selected: boolean;
  /** Backend sends these (specs/0074 W1); not yet normalized — see W1b. */
  lastSyncedAt?: string | null;
  lastError?: string | null;
}

/** Connection status for the Google Calendar provider (Settings card state). */
export interface GoogleCalendarStatus {
  /** Whether an OAuth client id is baked into this build (`NIXON_GOOGLE_CLIENT_ID`). */
  configured: boolean;
  connected: boolean;
  email: string | null;
  /** The grant lapsed; nothing syncs until reconnect (backend sends it; not yet normalized). */
  authRequired?: boolean;
  lastSyncedAt: string | null; // ISO-8601, or null when never synced (newest SELECTED calendar)
  calendars: GoogleCalendarListEntry[];
}

/** Result of a connect attempt — never-throw, so failures carry the message here. */
export type GoogleCalendarConnectResult =
  | { ok: true; email: string }
  | { ok: false; error: string };

/**
 * Probed best-effort enrichment capabilities for the connected Google account
 * (specs/0038 WS3, ADR-0010 amendment). Each flag is `null` until the connect-time
 * probe runs (fire-and-forget, populates a moment after connect) and `false` when the
 * org's directory/group policy denied it at runtime — a granted scope is NOT access.
 *   - `canExpandGroups`: distribution lists flatten to their individual members.
 *   - `canFetchPhotos`: attendee directory photos are downloaded (read-only).
 *   - `probedAt`: RFC3339 UTC of the last probe, or `null` when never probed.
 */
export interface GoogleCapabilities {
  canExpandGroups: boolean | null;
  canFetchPhotos: boolean | null;
  probedAt: string | null;
}

/** All-null capabilities: not connected, or the probe hasn't run yet ("checking…"). */
function unprobedCapabilities(): GoogleCapabilities {
  return { canExpandGroups: null, canFetchPhotos: null, probedAt: null };
}

/** Normalize a raw capabilities payload into the pinned tri-state shape. */
function normalizeCapabilities(raw: unknown): GoogleCapabilities {
  if (!raw || typeof raw !== 'object') return unprobedCapabilities();
  const r = raw as Partial<GoogleCapabilities>;
  const bool = (v: unknown): boolean | null => (typeof v === 'boolean' ? v : null);
  return {
    canExpandGroups: bool(r.canExpandGroups),
    canFetchPhotos: bool(r.canFetchPhotos),
    probedAt: typeof r.probedAt === 'string' ? r.probedAt : null,
  };
}

/** Safe status when the backend is unavailable: feature renders as not configured. */
function fallbackStatus(): GoogleCalendarStatus {
  return { configured: false, connected: false, email: null, lastSyncedAt: null, calendars: [] };
}

function errorMessage(err: unknown): string {
  if (typeof err === 'string') return err;
  if (err instanceof Error) return err.message;
  return 'Something went wrong. Please try again.';
}

/** Defensively normalize whatever the backend returns into the pinned shape. */
function normalizeStatus(raw: unknown): GoogleCalendarStatus {
  if (!raw || typeof raw !== 'object') return fallbackStatus();
  const r = raw as Partial<GoogleCalendarStatus>;
  const calendars = Array.isArray(r.calendars)
    ? r.calendars
        .filter(
          (c): c is GoogleCalendarListEntry =>
            !!c && typeof c === 'object' && typeof (c as GoogleCalendarListEntry).id === 'string',
        )
        .map((c) => ({
          id: c.id,
          summary: typeof c.summary === 'string' && c.summary ? c.summary : c.id,
          // DB default is selected=1, so treat anything but an explicit false as on.
          selected: c.selected !== false,
        }))
    : [];
  return {
    configured: r.configured === true,
    connected: r.connected === true,
    email: typeof r.email === 'string' ? r.email : null,
    lastSyncedAt: typeof r.lastSyncedAt === 'string' ? r.lastSyncedAt : null,
    calendars,
  };
}

/**
 * Read the Google Calendar connection status. Never throws — while the backend
 * command doesn't exist yet (or errors), returns the not-configured fallback so
 * the Settings card renders the inert state instead of breaking.
 */
export async function getGoogleCalendarStatus(): Promise<GoogleCalendarStatus> {
  try {
    return normalizeStatus(await invoke<GoogleCalendarStatus>('api_google_calendar_status'));
  } catch (err) {
    console.warn('[googleCalendar] getGoogleCalendarStatus failed:', err);
    return fallbackStatus();
  }
}

/**
 * Run the full OAuth connect flow (opens the browser; the backend waits up to
 * 5 minutes for consent). Never throws — failures/cancellations come back as
 * `{ ok: false, error }` with the backend's user-friendly message.
 */
export async function connectGoogleCalendar(): Promise<GoogleCalendarConnectResult> {
  try {
    const result = await invoke<{ email: string }>('api_google_calendar_connect');
    return { ok: true, email: typeof result?.email === 'string' ? result.email : '' };
  } catch (err) {
    console.warn('[googleCalendar] connectGoogleCalendar failed:', err);
    return { ok: false, error: errorMessage(err) };
  }
}

/**
 * Disconnect: backend revokes the token (best-effort), deletes it from the
 * Keychain, and purges the local Google event cache. Never throws; returns
 * whether the disconnect succeeded.
 */
export async function disconnectGoogleCalendar(): Promise<boolean> {
  try {
    await invoke('api_google_calendar_disconnect');
    return true;
  } catch (err) {
    console.warn('[googleCalendar] disconnectGoogleCalendar failed:', err);
    return false;
  }
}

/**
 * Toggle whether one calendar syncs. Never throws; returns success so the UI
 * can revert an optimistic checkbox. NOTE: the invoke arg keys MUST be camelCase
 * (`calendarId`) — a snake_case key silently becomes None on the Rust side.
 */
export async function setGoogleCalendarSelected(
  calendarId: string,
  selected: boolean,
): Promise<boolean> {
  try {
    await invoke('api_google_calendar_set_calendar_selected', { calendarId, selected });
    return true;
  } catch (err) {
    console.warn('[googleCalendar] setGoogleCalendarSelected failed:', err);
    return false;
  }
}

/**
 * Bulk form of `setGoogleCalendarSelected` for the Settings "Select all / none"
 * actions (specs/0041 WS5). One backend command instead of N client-batched
 * invokes: each per-calendar enable runs a full sync pass on the Rust side, so
 * batching there keeps Select-all to a single sync. Never throws; returns
 * success so the UI can revert an optimistic bulk toggle.
 */
export async function setGoogleCalendarsSelected(
  calendarIds: string[],
  selected: boolean,
): Promise<boolean> {
  try {
    await invoke('api_google_calendar_set_calendars_selected', { calendarIds, selected });
    return true;
  } catch (err) {
    console.warn('[googleCalendar] setGoogleCalendarsSelected failed:', err);
    return false;
  }
}

/**
 * Read the connected account's probed enrichment capabilities (specs/0038 WS3).
 * Instant/local — no network. Never throws: returns all-null ("checking…" / not
 * connected) when the backend command is unavailable or errors.
 */
export async function getGoogleCapabilities(): Promise<GoogleCapabilities> {
  try {
    return normalizeCapabilities(await invoke<GoogleCapabilities>('api_google_capabilities'));
  } catch (err) {
    console.warn('[googleCalendar] getGoogleCapabilities failed:', err);
    return unprobedCapabilities();
  }
}

/**
 * Trigger a manual sync. Never throws; returns whether the invoke resolved.
 * Boolean shim kept for the current UI — the backend now returns a
 * GoogleCalendarSyncOutcome, and resolving does NOT mean a pass ran (W1b).
 */
export async function syncGoogleCalendarNow(): Promise<boolean> {
  try {
    await invoke<GoogleCalendarSyncOutcome>('api_google_calendar_sync_now');
    return true;
  } catch (err) {
    console.warn('[googleCalendar] syncGoogleCalendarNow failed:', err);
    return false;
  }
}

/**
 * Tri-state of one best-effort capability for the Settings surface:
 *   - `'checking'` — not probed yet (flag `null`); the probe is fire-and-forget and
 *     populates a moment after connect, so the UI shows "checking…" and re-fetches.
 *   - `'on'` — the org allows it at runtime (flag `true`).
 *   - `'off'` — consented but denied by org policy (flag `false`).
 */
export type CapabilityState = 'checking' | 'on' | 'off';

/** Map a tri-state capability flag to its display state. `null` ⇒ still checking. */
export function capabilityState(flag: boolean | null): CapabilityState {
  if (flag === true) return 'on';
  if (flag === false) return 'off';
  return 'checking';
}

/** Whether any capability is still unprobed — drives the "checking…"/re-fetch loop. */
export function capabilitiesPending(caps: GoogleCapabilities): boolean {
  return caps.canExpandGroups === null || caps.canFetchPhotos === null;
}

/**
 * "Last synced just now" / "Last synced 5m ago" / "… 3h ago" / "… 2d ago", or
 * "Not synced yet" for null/unparseable input.
 */
export function formatLastSynced(lastSyncedAt: string | null, now: Date = new Date()): string {
  if (!lastSyncedAt) return 'Not synced yet';
  const synced = new Date(lastSyncedAt);
  if (Number.isNaN(synced.getTime())) return 'Not synced yet';
  const diffMin = Math.floor((now.getTime() - synced.getTime()) / 60000);
  if (diffMin < 1) return 'Last synced just now';
  if (diffMin < 60) return `Last synced ${diffMin}m ago`;
  const hours = Math.floor(diffMin / 60);
  if (hours < 24) return `Last synced ${hours}h ago`;
  return `Last synced ${Math.floor(hours / 24)}d ago`;
}
