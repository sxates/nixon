import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import PersonDetailsPage from '@/app/person-details/page';
import type { Person } from '@/types';

// specs/0038 dogfood feedback #3 — the dedicated person-detail page. These lock:
// (a) it loads the person via api_get_person and renders the three tabs (Summary default,
// Recent meetings, Voice Samples), (b) switching to a tab mounts its content, and
// (c) inline-editing the name persists via api_update_person.

const pushMock = vi.fn();
const backMock = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock, back: backMock }),
  useSearchParams: () => ({ get: (k: string) => (k === 'id' ? 'p1' : null) }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';

const invokeMock = vi.mocked(invoke);

const PERSON: Person = {
  id: 'p1',
  displayName: 'Alice Chen',
  email: 'alice@acme.io',
  role: 'Engineer',
  notes: null,
  voiceprintOptOut: false,
  starred: false,
  createdAt: '',
  updatedAt: '',
};

function routeInvoke(person: Person | null = PERSON) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_get_person':
        return Promise.resolve(person);
      case 'api_recent_meetings_with_person':
        return Promise.resolve([]);
      case 'api_list_person_voiceprints':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
}

describe('PersonDetailsPage (specs/0038 dogfood feedback #3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('loads the person and defaults to the Summary tab', async () => {
    routeInvoke();
    render(<PersonDetailsPage />);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_get_person', { id: 'p1' }),
    );
    // Name renders (as the inline-editable button) and the summary CTA is present.
    expect(await screen.findByLabelText('Person name')).toBeInTheDocument();
    expect(await screen.findByText('Summarize recent activity')).toBeInTheDocument();
  });

  it('switches tabs — Recent meetings then Voice Samples mount their content', async () => {
    routeInvoke();
    const user = userEvent.setup();
    render(<PersonDetailsPage />);
    await screen.findByLabelText('Person name');

    await user.click(screen.getByRole('tab', { name: 'Recent meetings' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_recent_meetings_with_person', {
        personId: 'p1',
      }),
    );
    expect(await screen.findByText(/No meetings with Alice Chen yet/)).toBeInTheDocument();

    await user.click(screen.getByRole('tab', { name: 'Voice Samples' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_list_person_voiceprints', {
        personId: 'p1',
      }),
    );
    expect(await screen.findByText('No voice samples yet')).toBeInTheDocument();
  });

  it('persists an inline name edit via api_update_person', async () => {
    routeInvoke();
    render(<PersonDetailsPage />);

    // Click the name to enter edit mode, change it, commit with Enter.
    const nameButton = await screen.findByLabelText('Person name');
    fireEvent.click(nameButton);
    const input = await screen.findByLabelText('Person name');
    fireEvent.change(input, { target: { value: 'Alice Cheng' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_update_person', {
        id: 'p1',
        displayName: 'Alice Cheng',
        email: 'alice@acme.io',
        role: 'Engineer',
        notes: null,
      }),
    );
  });

  it('shows a friendly not-found state when the person is gone', async () => {
    routeInvoke(null);
    render(<PersonDetailsPage />);
    expect(await screen.findByText('That person no longer exists.')).toBeInTheDocument();
  });
});
