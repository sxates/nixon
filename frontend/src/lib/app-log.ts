/**
 * Write a frontend message into the app's log file (`api_log_frontend`).
 *
 * Nixon's hardest bugs live in frontend orchestration — the deferred-backlog drain
 * sequences retranscribe → diarize → summarize entirely in TypeScript — and none of it
 * reached `~/Library/Logs/<bundle-id>/Nixon.log`. When a recording's automatic processing
 * stopped dead after the retranscription step (2026-09-21), the log showed the Rust work
 * succeeding and then nothing at all, and the step that gave up could not be identified.
 *
 * `console.log` does not solve this: by the time anyone asks, the devtools console is gone,
 * and a bundled build has no console to look at. These lines land in the file the user can
 * send, tagged `[fe:<scope>]`.
 *
 * Every function here is fire-and-forget and swallows its own errors — instrumentation must
 * never introduce a failure path into the flow it is observing.
 */

import { invoke } from '@tauri-apps/api/core';

type Level = 'debug' | 'info' | 'warn' | 'error';

function write(level: Level, scope: string, message: string): void {
  void invoke('api_log_frontend', { level, scope, message }).catch(() => {
    /* a diagnostic that fails must stay silent — never surface as an app error */
  });
}

/** Routine progress: which step ran, what it decided. */
export function logInfo(scope: string, message: string): void {
  write('info', scope, message);
}

/** Something gave up, was skipped, or took a fallback — the lines an investigation wants. */
export function logWarn(scope: string, message: string): void {
  write('warn', scope, message);
}

export function logError(scope: string, message: string): void {
  write('error', scope, message);
}
