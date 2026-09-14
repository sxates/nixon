import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import PeoplePage from '@/app/people/page';
import type { Person } from '@/types';

// specs/0038 dogfood feedback #3 — the People directory now NAVIGATES to a dedicated
// person-detail page on row click (it used to expand an inline panel). These lock:
// (a) clicking a row pushes /person-details?id=…, and (b) the nested star control stops
// propagation so it toggles the star instead of navigating.

const pushMock = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';

const invokeMock = vi.mocked(invoke);

const PEOPLE: Person[] = [
  {
    id: 'p1',
    displayName: 'Alice Chen',
    email: 'alice@acme.io',
    role: 'Engineer',
    notes: null,
    voiceprintOptOut: false,
    starred: false,
    createdAt: '',
    updatedAt: '',
  },
];

function routeInvoke() {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_list_people_ranked':
        return Promise.resolve(PEOPLE);
      default:
        return Promise.resolve(undefined);
    }
  });
}

describe('PeoplePage (specs/0038 dogfood feedback #3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('navigates to the person-detail page when a row is clicked', async () => {
    routeInvoke();
    render(<PeoplePage />);

    const row = await screen.findByText('Alice Chen');
    fireEvent.click(row);

    expect(pushMock).toHaveBeenCalledWith('/person-details?id=p1');
  });

  it('does not navigate when the star control is clicked', async () => {
    routeInvoke();
    render(<PeoplePage />);

    await screen.findByText('Alice Chen');
    const star = screen.getByRole('button', { name: /Star person/i });
    fireEvent.click(star);

    // The star fires its own IPC and must NOT trigger navigation.
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_person_starred', {
        personId: 'p1',
        starred: true,
      }),
    );
    expect(pushMock).not.toHaveBeenCalled();
  });
});
