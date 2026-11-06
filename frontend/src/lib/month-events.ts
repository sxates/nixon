/**
 * Calendar events for the Meetings Month view (calendar connected only).
 *
 * The events come from the same per-day `api_get_day_agenda` reads the Agenda and Week
 * views use, so they carry the same backend filters (declined events, the per-calendar
 * toggles) and the same event↔recording matching. What's left to do here:
 *   - drop hidden (dismissed) events, as the agenda does;
 *   - drop the agenda's own recording rows and any event already recorded — the Month view
 *     draws recordings from `api_get_meetings_in_range`, and a meeting adopted from an
 *     event must appear ONCE;
 *   - drop the duplicates a multi-day read can produce (the same occurrence read twice).
 */

import type { DayAgendaItem } from '@/lib/day-agenda';

export function monthEventItems(
  agendas: DayAgendaItem[][],
  recordedIds: ReadonlySet<string>,
): DayAgendaItem[] {
  const seen = new Set<string>();
  const out: DayAgendaItem[] = [];
  for (const day of agendas) {
    for (const it of day) {
      if (it.dismissed || it.source === 'recording' || it.status?.recorded) continue;
      if (it.meetingId && recordedIds.has(it.meetingId)) continue;
      const key = `${it.id}|${it.startTime}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(it);
    }
  }
  return out;
}

/**
 * Read `dayKeys` one day at a time in small parallel batches. The first day goes alone:
 * on a Google account that read is the one that may run the stale-cache sync, and every
 * read after it then finds the cache fresh instead of queueing behind the same sync.
 */
export async function readAgendaDays(
  dayKeys: string[],
  read: (key: string) => Promise<DayAgendaItem[]>,
  batchSize = 7,
): Promise<DayAgendaItem[][]> {
  if (dayKeys.length === 0) return [];
  const out: DayAgendaItem[][] = [await read(dayKeys[0])];
  for (let i = 1; i < dayKeys.length; i += batchSize) {
    out.push(...(await Promise.all(dayKeys.slice(i, i + batchSize).map(read))));
  }
  return out;
}
