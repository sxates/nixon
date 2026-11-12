import { invoke } from '@tauri-apps/api/core';

/**
 * The macOS Privacy & Security panes Nixon deep-links to. Values are the
 * `x-apple.systempreferences:com.apple.preference.security?<pane>` anchors that
 * the Rust `open_system_settings` command appends. Audio Capture (the Core Audio
 * tap) lives under "Screen & System Audio Recording", hence `Privacy_ScreenCapture`.
 */
export const PRIVACY_PANE = {
  microphone: 'Privacy_Microphone',
  systemAudio: 'Privacy_ScreenCapture',
  calendar: 'Privacy_Calendars',
} as const;

export type PrivacyPane = keyof typeof PRIVACY_PANE;

/**
 * Open System Settings at one permission's pane. `preferencePane` is a required
 * argument of the Rust command: calling it without one rejects, which is why
 * "Open Settings" used to do nothing. Rejects when the command fails; callers
 * keep their own fallback copy.
 */
export function openPrivacySettings(pane: PrivacyPane): Promise<void> {
  return invoke<void>('open_system_settings', { preferencePane: PRIVACY_PANE[pane] });
}
