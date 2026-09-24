/**
 * specs/0077 — "calm motion": Low Power Mode on battery also stills the continuous
 * animations (rail reels, VU needles, the listening pulse, the HOLD blink). Measured while
 * recording, those kept WindowServer recompositing the window every frame — about 20–30% of
 * a core while Nixon was visible. The backend applies the same rule to the menu bar reel
 * (`power::calm_motion`).
 */
export function calmMotion(lowPowerOnBattery: boolean, onBattery: boolean): boolean {
  return lowPowerOnBattery && onBattery;
}

/** Dispatched on `window` by the Low Power Mode toggle after it saves; `detail` is the new value. */
export const LOW_POWER_PREF_EVENT = 'nixon:low-power-pref';
