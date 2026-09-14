import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

// specs/0057 Task 7 — About panel re-skin: an engraved "NIXON" wordmark with a
// 10px rule beneath it, replacing the old plain "Nixon" heading. The version
// (from Tauri's getVersion) still renders below it.

const { getVersion } = vi.hoisted(() => ({ getVersion: vi.fn() }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion }));

import { About } from '@/components/About';

describe('About', () => {
  it('renders the NIXON wordmark and the version from getVersion', async () => {
    getVersion.mockResolvedValue('1.20.0');
    render(<About />);

    expect(screen.getByText('NIXON')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText(/v1\.20\.0/)).toBeInTheDocument());
  });
});
