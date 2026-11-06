import { useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { localDateKey } from '@/lib/today-timeline';
import {
  DEFAULT_VIEW,
  effectiveView,
  readStoredView,
  storeView,
  viewFromParam,
  type MeetingsView,
} from '@/lib/meetings-view';

/**
 * The Meetings page's view: `?view=` → the remembered view (migrated once from the old
 * Today / All meetings keys) → Agenda. Every switch is remembered, and so is a view opened
 * from a deep link — it is the view you last used either way.
 *
 * `view` is the CHOICE; `shown` is what renders: without a calendar the Agenda/Week
 * choice falls back to the List (the choice itself is kept, so reconnecting a calendar
 * brings it back).
 *
 * Must be rendered under a Suspense boundary (`useSearchParams`).
 */
export function useMeetingsView(calendarConnected: boolean | null) {
  const searchParams = useSearchParams();
  const [view, setView] = useState<MeetingsView>(() => {
    const fromUrl = viewFromParam(searchParams.get('view'));
    if (fromUrl) {
      storeView(fromUrl);
      return fromUrl;
    }
    return readStoredView() ?? DEFAULT_VIEW;
  });

  const chooseView = useCallback((next: MeetingsView) => {
    storeView(next);
    setView(next);
  }, []);

  return { view, shown: effectiveView(view, calendarConnected), chooseView };
}

/**
 * Mirror the view and the viewed day into the URL (`?view=&day=`), so the back button out
 * of a meeting lands on the same view and day you left (specs/0038 feedback). `replace`
 * keeps this off the history stack. The default — today's agenda — is a bare `/`.
 *
 * It mirrors the CHOICE, not the no-calendar fallback: writing `?view=list` for a fallback
 * would make coming back through that URL overwrite the remembered Agenda.
 */
export function useMeetingsUrlSync(view: MeetingsView, shown: MeetingsView, viewDate: string) {
  const router = useRouter();
  useEffect(() => {
    const params = new URLSearchParams();
    if (view !== 'agenda') params.set('view', view);
    if ((shown === 'agenda' || shown === 'week') && viewDate !== localDateKey()) {
      params.set('day', viewDate);
    }
    const qs = params.toString();
    router.replace(qs ? `/?${qs}` : '/', { scroll: false });
  }, [view, shown, viewDate, router]);
}
