import { useEffect, useState } from 'react';

/**
 * Below this window width the sidebar stops taking a 256px column (owner feedback
 * 2026-09-23: at narrow sizes the expanded panel left too little room for the page). The
 * rail stays; expanding it from there overlays the page instead of pushing it.
 */
export const NARROW_WINDOW_QUERY = '(max-width: 899px)';

/** Whether the window is currently narrower than the sidebar breakpoint. */
export function useNarrowWindow(): boolean {
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia?.(NARROW_WINDOW_QUERY);
    if (!mq) return;
    setNarrow(mq.matches);
    const onChange = (e: MediaQueryListEvent) => setNarrow(e.matches);
    mq.addEventListener?.('change', onChange);
    return () => mq.removeEventListener?.('change', onChange);
  }, []);
  return narrow;
}
