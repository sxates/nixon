/**
 * specs/0057 Plan 2 Task 7 (review round 1) — the error-alert modal takes ONE string, but
 * some producers have a real heading to show (a failed START names the device problem:
 * "Microphone Not Available"). Convention: an optional first line is the title, everything
 * after the first newline is the body. A message with no newline is a plain body under the
 * modal's historical heading, so every pre-existing caller (transcript/transcription errors,
 * which send a single sentence) renders exactly as before.
 */
export const DEFAULT_ERROR_ALERT_TITLE = 'Recording Stopped';

export function splitErrorAlert(message: string | undefined | null): { title: string; body: string } {
  const text = message ?? '';
  const nl = text.indexOf('\n');
  if (nl === -1) return { title: DEFAULT_ERROR_ALERT_TITLE, body: text };
  const title = text.slice(0, nl).trim();
  const body = text.slice(nl + 1);
  // A leading blank line is not a title — keep the default heading rather than an empty one.
  if (!title) return { title: DEFAULT_ERROR_ALERT_TITLE, body };
  return { title, body };
}
