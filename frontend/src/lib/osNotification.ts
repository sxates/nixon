/**
 * OS-level (native macOS) notification helper (spec 0008).
 *
 * Why this exists: in-app sonner toasts only appear when the Nixon window is
 * visible/focused. For prompts that must reach the user while Nixon is
 * backgrounded (Zoom-meeting-detected in P1, calendar alerts in P2), we send a
 * real macOS Notification Center notification via the Tauri notification plugin
 * (`@tauri-apps/plugin-notification`).
 *
 * This wraps three concerns so callers don't have to repeat them:
 *   1. Permission — `isPermissionGranted` / `requestPermission`, cached.
 *   2. Actionable buttons — a single registered action type ("Record" / "Ignore")
 *      via `registerActionTypes`, wired once.
 *   3. Routing — a module-level registry maps each sent notification (by id) to
 *      its `onRecord` / `onIgnore` / `onClick` callbacks. A single `onAction`
 *      listener (set up lazily, once) dispatches the pressed button / body-tap to
 *      the right callback, so the recording logic stays in the React component
 *      that owns `handleRecordingToggle`.
 *
 * Graceful degradation: if the plugin isn't available, permission is denied, or
 * sending throws (common in a bare `cargo run` dev binary — actionable macOS
 * notifications generally require a bundled, code-signed `.app`), `notify()`
 * returns `false` so the caller can fall back to an in-app toast and lose
 * nothing.
 *
 * Reusability note (P2): keep this generic — `notify()` takes title/body/actions
 * and is not Zoom-specific. Calendar alerts will reuse it as-is.
 */

import type {
  isPermissionGranted as IsPermissionGrantedFn,
  requestPermission as RequestPermissionFn,
  registerActionTypes as RegisterActionTypesFn,
  sendNotification as SendNotificationFn,
  onAction as OnActionFn,
} from '@tauri-apps/plugin-notification';

/** Action type id shared by every "Record this?" prompt. */
const RECORD_PROMPT_ACTION_TYPE = 'nixon-record-prompt';
const ACTION_RECORD = 'record';
const ACTION_IGNORE = 'ignore';

/**
 * Identifiers macOS sends for a body tap (vs a specific button). The plugin
 * forwards Apple's default-action identifier; older/newer builds have used a few
 * spellings, so we treat any of these as "clicked the notification itself".
 */
const DEFAULT_TAP_IDS = new Set([
  'tap',
  'default',
  'com.apple.UNNotificationDefaultActionIdentifier',
]);

export interface NotifyCallbacks {
  /** User pressed the "Record" button. */
  onRecord?: () => void;
  /** User pressed the "Ignore" button. */
  onIgnore?: () => void;
  /** User clicked the notification body (not a button). */
  onClick?: () => void;
}

export interface NotifyOptions extends NotifyCallbacks {
  title: string;
  body: string;
  /**
   * When true (default), attach the Record/Ignore action buttons. Set false for
   * a plain informational notification.
   */
  actionable?: boolean;
}

// ---------------------------------------------------------------------------
// Module state (all lazily initialized so importing this file is side-effect-free)
// ---------------------------------------------------------------------------

type NotifModule = {
  isPermissionGranted: typeof IsPermissionGrantedFn;
  requestPermission: typeof RequestPermissionFn;
  registerActionTypes: typeof RegisterActionTypesFn;
  sendNotification: typeof SendNotificationFn;
  onAction: typeof OnActionFn;
};

let modulePromise: Promise<NotifModule | null> | null = null;
let permissionGranted: boolean | null = null;
let actionTypesRegistered = false;
let actionListenerWired = false;

/** id → callbacks for in-flight notifications we sent. */
const callbackRegistry = new Map<number, NotifyCallbacks>();
let nextNotificationId = 1;

async function loadModule(): Promise<NotifModule | null> {
  if (!modulePromise) {
    modulePromise = import('@tauri-apps/plugin-notification')
      .then((m) => m as unknown as NotifModule)
      .catch((err) => {
        console.warn('[osNotification] notification plugin unavailable:', err);
        return null;
      });
  }
  return modulePromise;
}

/**
 * Ensure we have notification permission, requesting it once if needed.
 * Returns true only if granted. Result is cached for the session.
 */
async function ensurePermission(mod: NotifModule): Promise<boolean> {
  if (permissionGranted !== null) return permissionGranted;
  try {
    let granted = await mod.isPermissionGranted();
    console.log('[osNotification] isPermissionGranted:', granted);
    if (!granted) {
      const result = await mod.requestPermission();
      console.log('[osNotification] requestPermission result:', result);
      granted = result === 'granted';
    }
    permissionGranted = granted;
  } catch (err) {
    console.warn('[osNotification] permission check failed:', err);
    permissionGranted = false;
  }
  return permissionGranted;
}

/**
 * Proactively ensure notification permission at app startup (spec 0008, #4).
 *
 * Why: previously permission was requested lazily inside `notify()` — i.e.
 * mid-meeting, when Nixon is backgrounded and the user is in Zoom. The macOS
 * permission dialog is easy to miss there, and for a fresh bundle id
 * (`ai.vinyl.app.debug`) the plugin's `isPermissionGranted` is NOT auto-granted
 * despite the Rust side reporting so — so `notify()` silently returns false and
 * the prompt is lost. Calling this once on launch surfaces the dialog up front.
 *
 * Returns the resulting granted boolean (false if the plugin is unavailable).
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  const mod = await loadModule();
  if (!mod) {
    console.warn('[osNotification] ensureNotificationPermission: plugin unavailable');
    return false;
  }
  return ensurePermission(mod);
}

/**
 * Read whether OS notifications are currently granted, WITHOUT prompting.
 * Returns null if the plugin is unavailable or the check throws. Does not use the
 * session cache so the Settings UI always reflects the live OS state.
 */
export async function getNotificationPermission(): Promise<boolean | null> {
  const mod = await loadModule();
  if (!mod) return null;
  try {
    return await mod.isPermissionGranted();
  } catch (err) {
    console.warn('[osNotification] getNotificationPermission failed:', err);
    return null;
  }
}

/**
 * Explicitly request notification permission (for a user-initiated Settings
 * button). Unlike `ensureNotificationPermission`, this bypasses the session cache
 * and always asks the plugin, then updates the cache with the fresh result.
 * Returns the granted boolean (false if unavailable / denied / threw).
 */
export async function requestNotificationPermission(): Promise<boolean> {
  const mod = await loadModule();
  if (!mod) return false;
  try {
    const result = await mod.requestPermission();
    console.log('[osNotification] requestNotificationPermission result:', result);
    const granted = result === 'granted';
    permissionGranted = granted;
    return granted;
  } catch (err) {
    console.warn('[osNotification] requestNotificationPermission failed:', err);
    return false;
  }
}

/** Register the Record/Ignore action type once. Best-effort. */
async function ensureActionTypes(mod: NotifModule): Promise<void> {
  if (actionTypesRegistered) return;
  try {
    await mod.registerActionTypes([
      {
        id: RECORD_PROMPT_ACTION_TYPE,
        actions: [
          // `foreground: true` brings Nixon to the front when Record is pressed.
          { id: ACTION_RECORD, title: 'Record', foreground: true },
          { id: ACTION_IGNORE, title: 'Ignore', destructive: true },
        ],
      },
    ]);
    actionTypesRegistered = true;
  } catch (err) {
    // Not fatal — the notification can still show without buttons.
    console.warn('[osNotification] registerActionTypes failed:', err);
  }
}

/**
 * Wire the single global `onAction` listener once. It dispatches the pressed
 * button / body-tap to the callbacks registered for that notification id.
 */
async function ensureActionListener(mod: NotifModule): Promise<void> {
  if (actionListenerWired) return;
  actionListenerWired = true;
  try {
    await mod.onAction((payload) => {
      // The runtime payload carries the notification `id` plus the chosen
      // `actionId` (the plugin's type only models `Options`, so read loosely).
      const p = payload as { id?: number; actionId?: string };
      const id = p.id;
      const actionId = p.actionId;
      if (typeof id !== 'number') return;

      const cbs = callbackRegistry.get(id);
      if (!cbs) return;
      callbackRegistry.delete(id);

      if (actionId === ACTION_RECORD) {
        cbs.onRecord?.();
      } else if (actionId === ACTION_IGNORE) {
        cbs.onIgnore?.();
      } else if (actionId === undefined || DEFAULT_TAP_IDS.has(actionId)) {
        // Body tap (no specific button): treat as "open it".
        cbs.onClick?.();
      }
    });
  } catch (err) {
    console.warn('[osNotification] onAction listener failed to register:', err);
    actionListenerWired = false; // allow a later retry
  }
}

/**
 * Send a native OS notification. Returns true if it was dispatched, false if it
 * couldn't be (plugin missing, permission denied, or send threw) — in which case
 * the caller should fall back to an in-app prompt.
 */
export async function notify(options: NotifyOptions): Promise<boolean> {
  const { title, body, actionable = true, onRecord, onIgnore, onClick } = options;

  const mod = await loadModule();
  if (!mod) {
    console.warn('[osNotification] notify: plugin unavailable, falling back');
    return false;
  }

  const granted = await ensurePermission(mod);
  if (!granted) {
    console.warn('[osNotification] notify: permission not granted, falling back');
    return false;
  }

  // Set up action plumbing only when we actually need buttons.
  if (actionable) {
    await ensureActionTypes(mod);
    await ensureActionListener(mod);
  }

  const id = nextNotificationId++;
  if (onRecord || onIgnore || onClick) {
    callbackRegistry.set(id, { onRecord, onIgnore, onClick });
    // Don't let a never-actioned notification leak its callbacks forever.
    setTimeout(() => callbackRegistry.delete(id), 60 * 60 * 1000);
  }

  try {
    mod.sendNotification({
      id,
      title,
      body,
      ...(actionable && actionTypesRegistered
        ? { actionTypeId: RECORD_PROMPT_ACTION_TYPE }
        : {}),
    });
    console.log('[osNotification] notify: sent OS notification', { id, actionable });
    return true;
  } catch (err) {
    console.warn('[osNotification] sendNotification failed:', err);
    callbackRegistry.delete(id);
    return false;
  }
}

/**
 * Bring the Nixon window to the front (used when a backgrounded notification is
 * clicked). Best-effort and safe to call even if window APIs are unavailable.
 */
export async function focusMainWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    await win.show().catch(() => {});
    await win.unminimize().catch(() => {});
    await win.setFocus().catch(() => {});
  } catch (err) {
    console.warn('[osNotification] focusMainWindow failed:', err);
  }
}
