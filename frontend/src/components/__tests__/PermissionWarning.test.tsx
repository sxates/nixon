import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';

const openPrivacySettings = vi.fn().mockResolvedValue(undefined);
vi.mock('@/lib/system-settings', () => ({
  openPrivacySettings: (...args: unknown[]) => openPrivacySettings(...args),
}));
vi.mock('@/hooks/usePlatform', () => ({ useIsLinux: () => false }));

import { PermissionWarning } from '@/components/PermissionWarning';

beforeEach(() => {
  openPrivacySettings.mockClear();
  Object.defineProperty(window.navigator, 'userAgent', {
    value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)',
    configurable: true,
  });
});

describe('PermissionWarning', () => {
  it('renders nothing when both permissions are granted', () => {
    const { container } = render(<PermissionWarning hasMicrophone hasSystemAudio onRecheck={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('is one warn Callout titled "Permissions required" with Recheck as its action', () => {
    const onRecheck = vi.fn();
    const { container } = render(
      <PermissionWarning hasMicrophone={false} hasSystemAudio={false} onRecheck={onRecheck} />,
    );
    const callout = container.querySelector('[data-tone="warn"]') as HTMLElement;
    expect(callout).not.toBeNull();
    expect(within(callout).getByText('Permissions required')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Recheck' }));
    expect(onRecheck).toHaveBeenCalled();
  });

  it('gives each missing permission its own outline Open-settings key, and no primary', () => {
    render(<PermissionWarning hasMicrophone={false} hasSystemAudio={false} onRecheck={vi.fn()} />);
    const mic = screen.getByRole('button', { name: 'Open Microphone Settings' });
    const sys = screen.getByRole('button', { name: 'Open Audio Capture Settings' });
    for (const b of [mic, sys, screen.getByRole('button', { name: 'Recheck' })]) {
      expect(b.className).toContain('border-input');
      expect(b.className).not.toContain('bg-brand');
    }
    fireEvent.click(mic);
    expect(openPrivacySettings).toHaveBeenCalledWith('microphone');
    fireEvent.click(sys);
    expect(openPrivacySettings).toHaveBeenCalledWith('systemAudio');
  });

  it('shows the granted permission as Allowed, with no action', () => {
    render(<PermissionWarning hasMicrophone hasSystemAudio={false} onRecheck={vi.fn()} />);
    expect(screen.getByText('Allowed')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open Microphone Settings' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Open Audio Capture Settings' })).toBeInTheDocument();
    // The corrected copy: Nixon captures system audio itself.
    expect(screen.getByText(/no extra audio driver or routing is needed/)).toBeInTheDocument();
    expect(screen.queryByText(/BlackHole/i)).toBeNull();
  });
});
