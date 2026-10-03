import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn(), info: vi.fn() }),
}));

import { SummaryPromptSettings } from '../SummaryPromptSettings';
import type { SummaryPrompt } from '@/lib/summary-prompts';

const make = (over: Partial<SummaryPrompt>): SummaryPrompt => ({
  id: 'x',
  name: 'X',
  body: 'body',
  sort_order: 0,
  is_default: false,
  extract_action_items: false,
  in_library: true,
  created_at: '2026-01-01',
  updated_at: '2026-01-01',
  ...over,
});

const DEFAULT = make({
  id: 'd',
  name: 'General',
  body: 'General notes',
  is_default: true,
  extract_action_items: true,
});
const OTHER = make({ id: 'o', name: 'Standup', body: 'Short bullets' });

let library: SummaryPrompt[];

const listCalls = () => invokeMock.mock.calls.filter((c) => c[0] === 'api_list_summary_prompts');

beforeEach(() => {
  invokeMock.mockReset();
  library = [DEFAULT, OTHER];
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_list_summary_prompts') return library;
    return undefined;
  });
});

describe('SummaryPromptSettings (specs/0079 W4)', () => {
  it('lists prompts, marks the default, and hides Delete/Set default on it', async () => {
    render(<SummaryPromptSettings />);
    expect(await screen.findByText('General')).toBeInTheDocument();
    expect(screen.getByText('Standup')).toBeInTheDocument();
    expect(screen.getAllByText('Default')).toHaveLength(1);
    expect(screen.getAllByText('Action items')).toHaveLength(1);
    expect(screen.getByText('General notes')).toHaveClass('line-clamp-2');
    expect(screen.queryByRole('button', { name: /delete general/i })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /set general as default/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /delete standup/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /set standup as default/i })).toBeInTheDocument();
  });

  it('shows the empty state', async () => {
    library = [];
    render(<SummaryPromptSettings />);
    expect(
      await screen.findByText(
        'No prompts yet. Create one to shape how your meeting notes are written.',
      ),
    ).toBeInTheDocument();
  });

  it('opens the editor from New prompt', async () => {
    render(<SummaryPromptSettings />);
    await screen.findByText('General');
    await userEvent.click(screen.getByRole('button', { name: /new prompt/i }));
    expect(await screen.findByLabelText('Name')).toBeInTheDocument();
  });

  it('Set as default invokes the command then refreshes', async () => {
    render(<SummaryPromptSettings />);
    await screen.findByText('Standup');
    await userEvent.click(screen.getByRole('button', { name: /set standup as default/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_default_summary_prompt', { id: 'o' }),
    );
    await waitFor(() => expect(listCalls()).toHaveLength(2));
  });

  it('delete flow: confirm then delete then refresh', async () => {
    render(<SummaryPromptSettings />);
    await screen.findByText('Standup');
    await userEvent.click(screen.getByRole('button', { name: /delete standup/i }));
    const dialog = await screen.findByRole('dialog');
    expect(invokeMock).not.toHaveBeenCalledWith('api_delete_summary_prompt', expect.anything());
    await userEvent.click(within(dialog).getByRole('button', { name: /^delete$/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_delete_summary_prompt', { id: 'o' }),
    );
    await waitFor(() => expect(listCalls()).toHaveLength(2));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});
