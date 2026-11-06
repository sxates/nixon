'use client';

import { useEffect, useState } from 'react';

/**
 * specs/0083 W2c — whether the window is on screen.
 *
 * The main window runs with `backgroundThrottling: "disabled"` so the wrap-up's handoff and
 * the meeting's events are handled while it is hidden. That also stops WebKit from parking
 * our own timers, so the JS-driven animation loops (the VU needles' rAF, the level feed, the
 * transcript typewriter) subscribe to this and stop scheduling while hidden — the way they
 * already stop for calm motion (specs/0077). CSS animations need nothing.
 */
export function isDocumentVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}

export function useDocumentVisible(): boolean {
  const [visible, setVisible] = useState(isDocumentVisible);
  useEffect(() => {
    const onChange = () => setVisible(isDocumentVisible());
    onChange();
    document.addEventListener('visibilitychange', onChange);
    return () => document.removeEventListener('visibilitychange', onChange);
  }, []);
  return visible;
}
