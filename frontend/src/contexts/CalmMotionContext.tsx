'use client';

/**
 * specs/0077 — tracks whether animations should be still (Low Power Mode is on and the Mac
 * is on battery), marks `<html data-calm-motion="1">` so globals.css can stop the CSS
 * animations, and exposes `useCalmMotion()` for the JS ones (the VU needles).
 *
 * Reads the power source and the setting at mount, then follows `power-source-changed`
 * and the Low Power Mode toggle's `LOW_POWER_PREF_EVENT`. Without a provider (tests,
 * isolated renders) `useCalmMotion()` is false, so nothing changes.
 */

import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import { calmMotion, LOW_POWER_PREF_EVENT } from '@/lib/calm-motion';

const CalmMotionContext = createContext(false);

export function useCalmMotion(): boolean {
  return useContext(CalmMotionContext);
}

export function CalmMotionProvider({ children }: { children: ReactNode }) {
  // Both default to the state that animates, so a failed read never stills the app.
  const [lowPower, setLowPower] = useState(false);
  const [onBattery, setOnBattery] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<{ onBattery?: boolean }>('api_get_power_state')
      .then((s) => !cancelled && setOnBattery(s?.onBattery === true))
      .catch((e) => console.warn('[calm-motion] power state unavailable:', e));
    invoke<{ low_power_on_battery?: boolean }>('get_recording_preferences')
      .then((p) => !cancelled && setLowPower(p?.low_power_on_battery !== false))
      .catch((e) => console.warn('[calm-motion] preferences unavailable:', e));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(
    () =>
      safeListen<{ onBattery: boolean }>('power-source-changed', (event) =>
        setOnBattery(event.payload.onBattery === true),
      ),
    [],
  );

  useEffect(() => {
    const onPref = (e: Event) => setLowPower((e as CustomEvent<boolean>).detail === true);
    window.addEventListener(LOW_POWER_PREF_EVENT, onPref);
    return () => window.removeEventListener(LOW_POWER_PREF_EVENT, onPref);
  }, []);

  const calm = calmMotion(lowPower, onBattery);
  useEffect(() => {
    const root = document.documentElement;
    if (calm) root.dataset.calmMotion = '1';
    else delete root.dataset.calmMotion;
  }, [calm]);

  return <CalmMotionContext.Provider value={calm}>{children}</CalmMotionContext.Provider>;
}
