import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, act, waitFor } from '@testing-library/react';

// specs/0077 — the provider marks <html data-calm-motion> while Low Power Mode is on and the
// Mac is on battery, and follows the power source and the setting live.
const h = vi.hoisted(() => ({
  onBattery: true,
  lowPower: true,
  powerHandler: null as null | ((e: { payload: { onBattery: boolean } }) => void),
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === 'api_get_power_state') return Promise.resolve({ onBattery: h.onBattery });
    if (cmd === 'get_recording_preferences')
      return Promise.resolve({ low_power_on_battery: h.lowPower });
    return Promise.resolve(undefined);
  }),
}));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (name: string, handler: (e: { payload: { onBattery: boolean } }) => void) => {
    if (name === 'power-source-changed') h.powerHandler = handler;
    return () => {};
  },
}));

import { CalmMotionProvider, useCalmMotion } from '@/contexts/CalmMotionContext';
import { LOW_POWER_PREF_EVENT } from '@/lib/calm-motion';

function Probe() {
  return <span data-testid="calm">{String(useCalmMotion())}</span>;
}
const calmAttr = () => document.documentElement.dataset.calmMotion;

beforeEach(() => {
  h.onBattery = true;
  h.lowPower = true;
  h.powerHandler = null;
  delete document.documentElement.dataset.calmMotion;
});

describe('CalmMotionProvider', () => {
  it('is calm on battery with Low Power Mode on, and clears when plugged in', async () => {
    const { getByTestId } = render(
      <CalmMotionProvider>
        <Probe />
      </CalmMotionProvider>,
    );
    await waitFor(() => expect(calmAttr()).toBe('1'));
    expect(getByTestId('calm').textContent).toBe('true');

    act(() => h.powerHandler?.({ payload: { onBattery: false } }));
    await waitFor(() => expect(calmAttr()).toBeUndefined());
    expect(getByTestId('calm').textContent).toBe('false');
  });

  it('follows the Low Power Mode setting when it is switched off', async () => {
    render(
      <CalmMotionProvider>
        <Probe />
      </CalmMotionProvider>,
    );
    await waitFor(() => expect(calmAttr()).toBe('1'));
    act(() => {
      window.dispatchEvent(new CustomEvent(LOW_POWER_PREF_EVENT, { detail: false }));
    });
    await waitFor(() => expect(calmAttr()).toBeUndefined());
  });

  it('stays animated on AC power', async () => {
    h.onBattery = false;
    const { getByTestId } = render(
      <CalmMotionProvider>
        <Probe />
      </CalmMotionProvider>,
    );
    await waitFor(() => expect(getByTestId('calm').textContent).toBe('false'));
    expect(calmAttr()).toBeUndefined();
  });
});
