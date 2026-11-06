/**
 * The Meetings page's view choice (Today and All meetings merged into one page).
 *
 * With a calendar connected the page offers Agenda / Week / Month / List; without one it
 * offers the old All meetings pair, Month / List. The choice is remembered under ONE key,
 * migrated once from the two keys the separate pages used.
 *
 * Pure: storage access goes through the guarded helpers below (private windows throw on
 * `localStorage`), so the page still renders when storage is unavailable.
 */

export type MeetingsView = 'agenda' | 'week' | 'month' | 'list';

/** Where the unified view is remembered. */
export const MEETINGS_VIEW_KEY = 'nixon.meetings.view';
/** The retired Today page's key: `'day' | 'week' | 'list'` (its `'list'` was a day list). */
export const LEGACY_TODAY_VIEW_KEY = 'nixon.today.viewMode';
/** The retired All meetings page's key: `'month' | 'list'`. */
export const LEGACY_MEETINGS_VIEW_KEY = 'nixon.meetings.viewMode';

/** First visit, calendar connected: today's agenda. */
export const DEFAULT_VIEW: MeetingsView = 'agenda';
/** No calendar: the All meetings list, as it was. */
export const NO_CALENDAR_DEFAULT: MeetingsView = 'list';

export function isMeetingsView(v: unknown): v is MeetingsView {
  return v === 'agenda' || v === 'week' || v === 'month' || v === 'list';
}

/** Views that need a calendar: they only make sense with events to show. */
export function needsCalendar(view: MeetingsView): boolean {
  return view === 'agenda' || view === 'week';
}

/**
 * A `?view=` value, including the old pages' deep links: Today wrote `day` for its agenda,
 * and its `list` (a single-day list, retired) now lands on the List view.
 */
export function viewFromParam(v: string | null | undefined): MeetingsView | null {
  if (v === 'day') return 'agenda';
  return isMeetingsView(v) ? v : null;
}

/** What actually renders: a calendar-only view falls back to the list without a calendar. */
export function effectiveView(view: MeetingsView, calendarConnected: boolean | null): MeetingsView {
  return calendarConnected === false && needsCalendar(view) ? NO_CALENDAR_DEFAULT : view;
}

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* private window / quota — the in-memory choice still holds for this session */
  }
}

function remove(key: string): void {
  try {
    window.localStorage.removeItem(key);
  } catch {
    /* as above */
  }
}

/**
 * The remembered view, migrating once from the old keys when the new one is absent.
 *
 * Today was the home page, so its agenda/week choice wins; its retired day-list mode
 * carries no meaning on the new page and defers to All meetings' Month/List choice. The
 * old keys are removed after migrating so a later reset can't resurrect them.
 */
export function readStoredView(): MeetingsView | null {
  const current = read(MEETINGS_VIEW_KEY);
  if (isMeetingsView(current)) return current;

  const today = read(LEGACY_TODAY_VIEW_KEY);
  const meetings = read(LEGACY_MEETINGS_VIEW_KEY);
  let migrated: MeetingsView | null = null;
  if (today === 'day') migrated = 'agenda';
  else if (today === 'week') migrated = 'week';
  else if (meetings === 'month' || meetings === 'list') migrated = meetings;

  if (today !== null || meetings !== null) {
    if (migrated) write(MEETINGS_VIEW_KEY, migrated);
    remove(LEGACY_TODAY_VIEW_KEY);
    remove(LEGACY_MEETINGS_VIEW_KEY);
  }
  return migrated;
}

export function storeView(view: MeetingsView): void {
  write(MEETINGS_VIEW_KEY, view);
}

/** The `/meetings` redirect target: same query, defaulting to the List view it used to open on. */
export function meetingsRedirectTarget(search: string): string {
  const params = new URLSearchParams(search);
  if (!viewFromParam(params.get('view'))) params.set('view', 'list');
  return `/?${params.toString()}`;
}

/** One content width for every Meetings view (the month grid's), so switching views never
 *  moves the edges. Action items uses it too, so the two list pages line up. A literal class string so Tailwind's scan of src/lib keeps it. */
export const MEETINGS_CONTENT_WIDTH = 'max-w-[1080px]';
