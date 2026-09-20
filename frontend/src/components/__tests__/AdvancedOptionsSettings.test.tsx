import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// specs/0067 W0 — one switch, learned once, governing every section that Nixon can decide
// for you. The rule that matters is the exception: a value someone deliberately moved off
// the recommendation stays visible whether the switch is on or not, because hiding it would
// both conceal what the app is doing and remove the only way to undo it.

const { toggleShowAdvanced } = vi.hoisted(() => ({ toggleShowAdvanced: vi.fn() }));
let showAdvanced = false;
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ showAdvanced, toggleShowAdvanced }),
}));

import { AdvancedOptionsSettings, isAdvancedRowVisible } from '@/components/AdvancedOptionsSettings';

beforeEach(() => {
  vi.clearAllMocks();
  showAdvanced = false;
});

describe('isAdvancedRowVisible', () => {
  it('hides the expert controls by default, when the recommendation is in use', () => {
    expect(isAdvancedRowVisible({ showAdvanced: false, isRecommended: true })).toBe(false);
  });

  it('shows them when the switch is on', () => {
    expect(isAdvancedRowVisible({ showAdvanced: true, isRecommended: true })).toBe(true);
  });

  it('shows them for a non-recommended value even with the switch off', () => {
    // Someone on a cloud provider, or a model they picked themselves, must keep seeing it.
    expect(isAdvancedRowVisible({ showAdvanced: false, isRecommended: false })).toBe(true);
  });
});

describe('AdvancedOptionsSettings', () => {
  it('is off by default and explains what turning it on does', () => {
    render(<AdvancedOptionsSettings />);
    const control = screen.getByRole('switch', { name: 'Show advanced options' });
    expect(control).not.toBeChecked();
    expect(screen.getByText(/engine and model pickers/i)).toBeInTheDocument();
  });

  it('toggles through the config context, which persists it', () => {
    render(<AdvancedOptionsSettings />);
    fireEvent.click(screen.getByRole('switch', { name: 'Show advanced options' }));
    expect(toggleShowAdvanced).toHaveBeenCalledWith(true);
  });

  it('reflects the flag when it is already on', () => {
    showAdvanced = true;
    render(<AdvancedOptionsSettings />);
    expect(screen.getByRole('switch', { name: 'Show advanced options' })).toBeChecked();
  });
});
