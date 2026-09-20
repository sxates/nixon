/**
 * OS-level (macOS Notification Center) notifications — specs/0068.
 *
 * ## Why this file was rewritten
 *
 * It used to call `@tauri-apps/plugin-notification` directly, and could never have worked.
 * That plugin registers three commands on desktop — `notify`, `request_permission`,
 * `is_permission_granted`; `registerActionTypes` and `onAction` are **mobile-only**, so the
 * action buttons and the click routing threw on every macOS call and were swallowed by the
 * `try/catch` around them. `request_permission` returned `granted` without asking macOS, so
 * the system never showed its dialog. And delivery went through the deprecated
 * `NSUserNotificationCenter`, which macOS 26 no longer surfaces.
 *
 * Everything now goes through four Tauri commands over a native `UNUserNotificationCenter`
 * implementation (`src-tauri/src/notifications/macos/`), and a press comes back as a
 * `notification-action` event rather than a plugin callback — which is what lets a button
 * work while Nixon is in the background, the whole point of the feature.
 *
 * ## Routing
 *
 * Each `notify()` registers its callbacks under the notification's id. One `listen` for
 * `notification-action` dispatches the press to them. A press for an id we do not know —
 * a banner that outlived a reload — is dropped.
 *
 * ## Degradation
 *
 * `notify()` returns `false` rather than throwing when notifications are unavailable (the
 * dev build is not inside an `.app`, so macOS will not hand out a notification centre) or
 * permission was refused, so every caller can fall back to an in-app toast.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/** Categories, mirroring `notifications::macos` — a banner gets its buttons by naming one. */
export const CATEGORY_MEETING = 'nixon.meeting';
export const CATEGORY_RECORD = 'nixon.record';
export const CATEGORY_PLAIN = 'nixon.plain';

/** Action ids, mirroring `notifications::macos`. */
export const ACTION_JOIN = 'join';
export const ACTION_JOIN_AND_RECORD = 'join_and_record';
export const ACTION_RECORD = 'record';
/** The banner body itself was tapped (Apple's default action, normalized in Rust). */
export const ACTION_OPEN = 'open';

export type NotificationCategory =
  | typeof CATEGORY_MEETING
  | typeof CATEGORY_RECORD
  | typeof CATEGORY_PLAIN;

export type AuthorizationStatus =
  | 'not_determined'
  | 'denied'
  | 'authorized'
  | 'provisional'
  | 'unavailable';

export interface NotificationCapability {
  supported: boolean;
  /** Why not, in a sentence the Settings row prints as-is. */
  reason: string | null;
}

export interface NotifyCallbacks {
  /** "Join" — open the meeting, don't record. */
  onJoin?: () => void;
  /** "Join & Record" — open the meeting and start a recording bound to it. */
  onJoinAndRecord?: () => void;
  /** "Record" — the single button on a detected-call prompt. */
  onRecord?: () => void;
  /** The banner body was tapped. */
  onOpen?: () => void;
}

export interface NotifyOptions extends NotifyCallbacks {
  title: string;
  body: string;
  /** Defaults to `CATEGORY_PLAIN` — a banner with no buttons. */
  category?: NotificationCategory;
  /**
   * Stable id. Delivering the same id twice replaces the banner instead of stacking a
   * second one, which is what you want for "meeting in N minutes". Generated when omitted.
   */
  id?: string;
  /** Handed back verbatim on the action event; useful for logging and debugging. */
  userInfo?: Record<string, string>;
}

interface ActionEvent {
  actionId: string;
  notificationId: string;
  userInfo: Record<string, string>;
}

// ---------------------------------------------------------------------------
// Module state (lazy, so importing this file has no side effects)
// ---------------------------------------------------------------------------

let capabilityPromise: Promise<NotificationCapability> | null = null;
let listenerWired = false;
let nextId = 1;

const callbacks = new Map<string, NotifyCallbacks>();

/** An unpressed banner should not pin its callbacks forever. */
const CALLBACK_TTL_MS = 60 * 60 * 1000;

/**
 * Can this build deliver an OS notification? Cached: it cannot change within a session,
 * since it is decided by whether the running process is inside an `.app`.
 */
export async function getNotificationCapability(): Promise<NotificationCapability> {
  if (!capabilityPromise) {
    capabilityPromise = invoke<NotificationCapability>('notif_capability').catch((error) => {
      console.warn('[osNotification] capability check failed:', error);
      return { supported: false, reason: 'Notifications are unavailable in this build.' };
    });
  }
  return capabilityPromise;
}

/** Read the macOS authorization state. Never prompts. */
export async function getNotificationPermission(): Promise<AuthorizationStatus> {
  try {
    return await invoke<AuthorizationStatus>('notif_authorization_status');
  } catch (error) {
    console.warn('[osNotification] status read failed:', error);
    return 'unavailable';
  }
}

/**
 * Ask macOS for permission, showing its dialog. Only ever prompts once per install — after
 * that macOS returns the stored answer — so a refusal has to be undone in System Settings.
 */
export async function requestNotificationPermission(): Promise<boolean> {
  const { supported } = await getNotificationCapability();
  if (!supported) return false;
  try {
    return await invoke<boolean>('notif_request_authorization');
  } catch (error) {
    console.warn('[osNotification] permission request failed:', error);
    return false;
  }
}

/**
 * Make sure we are allowed to notify, asking the first time.
 *
 * Called at the point of first *use* rather than at launch, deliberately: asking on first
 * run would stack a third dialog behind microphone and audio-capture, where it is easy to
 * dismiss without reading. The Settings row offers the same request explicitly.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  const { supported } = await getNotificationCapability();
  if (!supported) return false;

  const status = await getNotificationPermission();
  if (status === 'authorized' || status === 'provisional') return true;
  if (status === 'not_determined') return requestNotificationPermission();
  return false;
}

/** Open System Settings → Notifications, the only way back from a refusal. */
export async function openNotificationSettings(): Promise<void> {
  try {
    await invoke('notif_open_system_settings');
  } catch (error) {
    console.warn('[osNotification] could not open notification settings:', error);
  }
}

/** Wire the single `notification-action` listener. */
async function ensureListener(): Promise<void> {
  if (listenerWired) return;
  listenerWired = true;
  try {
    await listen<ActionEvent>('notification-action', ({ payload }) => {
      const registered = callbacks.get(payload.notificationId);
      if (!registered) return;
      // Buttons are one-shot: the banner is gone once pressed.
      callbacks.delete(payload.notificationId);

      switch (payload.actionId) {
        case ACTION_JOIN:
          registered.onJoin?.();
          break;
        case ACTION_JOIN_AND_RECORD:
          registered.onJoinAndRecord?.();
          break;
        case ACTION_RECORD:
          registered.onRecord?.();
          break;
        case ACTION_OPEN:
          registered.onOpen?.();
          break;
        default:
          console.warn('[osNotification] unknown action:', payload.actionId);
      }
    });
  } catch (error) {
    console.warn('[osNotification] could not listen for notification actions:', error);
    listenerWired = false; // allow a later retry
  }
}

/**
 * Send a native OS notification. Returns true if it was handed to macOS, false if it could
 * not be — in which case the caller should fall back to an in-app prompt.
 */
export async function notify(options: NotifyOptions): Promise<boolean> {
  const {
    title,
    body,
    category = CATEGORY_PLAIN,
    userInfo = {},
    onJoin,
    onJoinAndRecord,
    onRecord,
    onOpen,
  } = options;

  const granted = await ensureNotificationPermission();
  if (!granted) {
    console.warn('[osNotification] not permitted; falling back to the in-app prompt');
    return false;
  }

  await ensureListener();

  const id = options.id ?? `nixon-${nextId++}`;
  if (onJoin || onJoinAndRecord || onRecord || onOpen) {
    callbacks.set(id, { onJoin, onJoinAndRecord, onRecord, onOpen });
    setTimeout(() => callbacks.delete(id), CALLBACK_TTL_MS);
  }

  try {
    await invoke('notif_deliver', { request: { id, title, body, category, userInfo } });
    return true;
  } catch (error) {
    console.warn('[osNotification] delivery failed:', error);
    callbacks.delete(id);
    return false;
  }
}

/**
 * Bring the Nixon window to the front. Best-effort and safe to call even if the window APIs
 * are unavailable.
 */
export async function focusMainWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    await win.show().catch(() => {});
    await win.unminimize().catch(() => {});
    await win.setFocus().catch(() => {});
  } catch (error) {
    console.warn('[osNotification] focusMainWindow failed:', error);
  }
}

/** Test seam: forget the cached capability and the registered callbacks. */
export function __resetNotificationStateForTests(): void {
  capabilityPromise = null;
  listenerWired = false;
  callbacks.clear();
  nextId = 1;
}
