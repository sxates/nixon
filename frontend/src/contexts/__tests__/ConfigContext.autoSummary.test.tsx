import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

// specs/0066 W1 (owner request): a summary when the recording stops is what people
// expect, so the toggle now defaults ON. The subtlety worth pinning is the other half —
// only an ABSENT key falls through to the default, so someone who deliberately turned it
// off keeps it off across the update. A naive `saved === 'true'` fallback would be
// indistinguishable from this on a fresh profile and wrong for that person.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn().mockReturnValue(() => {}) }));
vi.mock('@/services/configService', () => ({
  configService: {
    getModelConfig: vi.fn().mockResolvedValue(null),
    saveModelConfig: vi.fn().mockResolvedValue(undefined),
    getTranscriptConfig: vi.fn().mockResolvedValue(null),
    getRecordingPreferences: vi.fn().mockResolvedValue(null),
  },
}));

import { ConfigProvider, useConfig } from '@/contexts/ConfigContext';

function Probe() {
  const { isAutoSummary } = useConfig();
  return <span data-testid="auto">{String(isAutoSummary)}</span>;
}

const readDefault = () => {
  render(
    <ConfigProvider>
      <Probe />
    </ConfigProvider>,
  );
  return screen.getByTestId('auto').textContent;
};

describe('ConfigContext — auto-summary default (specs/0066 W1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it('is on for a profile that has never touched the switch', () => {
    expect(readDefault()).toBe('true');
  });

  it('stays off for someone who turned it off', () => {
    localStorage.setItem('isAutoSummary', 'false');
    expect(readDefault()).toBe('false');
  });

  it('stays on for someone who turned it on explicitly', () => {
    localStorage.setItem('isAutoSummary', 'true');
    expect(readDefault()).toBe('true');
  });
});
