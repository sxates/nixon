/**
 * Cap a list for display with an "expand for the rest" affordance (specs/0019 WS3.1).
 *
 * When collapsed, show at most `cap` items and report how many are hidden so the UI can
 * render a "+N more" control; when `expanded`, show everything. Pure and generic so it's
 * unit-testable and reusable (participant list, speaker pickers, …).
 */
export function splitForDisplay<T>(
  items: T[],
  cap: number,
  expanded: boolean,
): { shown: T[]; hidden: number } {
  if (expanded || cap <= 0 || items.length <= cap) {
    return { shown: items, hidden: 0 };
  }
  return { shown: items.slice(0, cap), hidden: items.length - cap };
}
