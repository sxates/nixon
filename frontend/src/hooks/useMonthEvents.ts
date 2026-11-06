import { useEffect, useState } from 'react';
import { readDayAgenda, type DayAgendaItem } from '@/lib/day-agenda';
import { buildMonthGrid, localDayKey } from '@/lib/month-grid';
import { readAgendaDays } from '@/lib/month-events';
import { safeListen } from '@/lib/safe-listen';

/**
 * The visible month grid's agenda, one array per grid day — read the same way the Week
 * view reads its seven days (`api_get_day_agenda` per day), so Month shows the events Week
 * shows. Read when the month changes and after a Google sync pass, not on a timer: a month
 * is up to 42 day reads. `enabled: false` (no calendar) reads nothing.
 */
export function useMonthEvents(year: number, monthIndex: number, enabled: boolean) {
  const [agendas, setAgendas] = useState<DayAgendaItem[][]>([]);

  useEffect(() => {
    if (!enabled) {
      setAgendas([]);
      return;
    }
    let cancelled = false;
    const keys = buildMonthGrid(year, monthIndex).map((cell) => localDayKey(cell.date));
    const load = async () => {
      const days = await readAgendaDays(keys, async (k) => (await readDayAgenda(k)).items);
      if (!cancelled) setAgendas(days);
    };
    void load();
    const dispose = safeListen('google-calendar-synced', () => void load());
    return () => {
      cancelled = true;
      dispose();
    };
  }, [year, monthIndex, enabled]);

  return agendas;
}
