import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

// specs/0085 Task F — the Settings panel for the Claude Code subscription provider.
// Nixon has no sign-in flow: it tests whether it can reach the service and says so in
// plain words. Save is only possible when the CLI is ready.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { CliProviderPanel, CliProviderSelectItems } from '@/components/CliProviderPanel';
import { Select, SelectContent, SelectTrigger, SelectValue } from '@/components/ui/select';

const BASE = {
  id: 'claude-code',
  displayName: 'Claude Code',
  minVersion: '2.1.290',
  models: [{ id: 'sonnet', label: 'Sonnet' }],
};
const READY = { ...BASE, state: 'ready', path: '/usr/local/bin/claude', version: '2.1.301' };

function arrange(status: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  invokeMock.mockImplementation((cmd: string, args?: any) => {
    if (cmd === 'api_cli_providers_status') return Promise.resolve([status]);
    if (extra[cmd] !== undefined) {
      const v = extra[cmd];
      return typeof v === 'function' ? (v as any)(args) : Promise.resolve(v);
    }
    return Promise.resolve(undefined);
  });
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe('CliProviderPanel (specs/0085)', () => {
  it('shows a ready Claude Code with its path, version, pinned model and privacy line', async () => {
    arrange(READY);
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    expect(await screen.findByText(/\/usr\/local\/bin\/claude/)).toBeInTheDocument();
    expect(screen.getByText(/2\.1\.301/)).toBeInTheDocument();
    expect(screen.getByText('Model: Sonnet')).toBeInTheDocument();
    expect(screen.queryByRole('combobox')).toBeNull(); // pinned: no picker
    expect(
      screen.getByText(/sent to Anthropic through Claude Code, under your Claude plan's terms/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/also sends your account email and basic details about your Mac with each request/),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled();
  });

  it('disables Save unless the CLI is ready', async () => {
    arrange({ ...BASE, state: 'needsSignIn', path: '/usr/local/bin/claude', version: '2.1.301' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    expect(
      await screen.findByText(
        "Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then Check again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    // No sign-in flow, no Terminal.
    expect(screen.queryByText(/terminal/i)).toBeNull();
  });

  it('offers Locate and install help when Claude Code is not found', async () => {
    arrange({ ...BASE, state: 'notFound' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    expect(await screen.findByRole('button', { name: /Locate/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /How to install/ }));
    expect(calls('open_external_url')).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('asks for an update when the CLI is too old', async () => {
    arrange({ ...BASE, state: 'tooOld', path: '/usr/local/bin/claude', version: '1.0.0' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    expect(await screen.findByText(/Update Claude Code, then Check again/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('shows a transient probe failure instead of the install or sign-in copy', async () => {
    arrange({ ...BASE, state: 'notFound', probeError: 'Claude Code took too long to answer.' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    expect(await screen.findByText('Claude Code took too long to answer.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Check again/ })).toBeInTheDocument();
    expect(screen.queryByText(/How to install/)).toBeNull();
    expect(screen.queryByText(/signed in to Claude Code/)).toBeNull();
  });

  it('shows the API-key billing note as muted secondary text', async () => {
    arrange({ ...READY, authNote: 'Claude Code is using an API key, so usage is billed to it.' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);
    expect(await screen.findByText(/billed to it/)).toBeInTheDocument();
  });

  it('Check again re-probes', async () => {
    arrange({ ...BASE, state: 'needsSignIn' });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);
    await screen.findByRole('button', { name: /Check again/ });

    arrange(READY);
    fireEvent.click(screen.getByRole('button', { name: /Check again/ }));

    await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled());
    expect(calls('api_cli_providers_status')).toHaveLength(2);
  });

  it('Locate opens the native picker, saves the chosen program, and shows a rejection inline', async () => {
    arrange({ ...BASE, state: 'notFound' }, {
      api_cli_provider_pick_binary: '/opt/tools/not-claude',
      api_cli_provider_set_path: () =>
        Promise.reject("That file can't be run as Claude Code. Choose the `claude` program itself."),
    });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Locate/ }));

    expect(await screen.findByText(/can't be run as Claude Code/)).toBeInTheDocument();
    expect(calls('api_cli_provider_pick_binary')).toHaveLength(1);
    expect(calls('api_cli_provider_set_path')[0][1]).toEqual({ id: 'claude-code', path: '/opt/tools/not-claude' });
  });

  it('Locate adopts the returned status on success', async () => {
    arrange({ ...BASE, state: 'notFound' }, {
      api_cli_provider_pick_binary: '/usr/local/bin/claude',
      api_cli_provider_set_path: READY,
    });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Locate/ }));

    await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled());
  });

  it('cancelling the picker changes nothing', async () => {
    arrange({ ...BASE, state: 'notFound' }, { api_cli_provider_pick_binary: null });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Locate/ }));
    await waitFor(() => expect(calls('api_cli_provider_pick_binary')).toHaveLength(1));
    expect(calls('api_cli_provider_set_path')).toHaveLength(0);
  });

  it('Use default clears a saved location', async () => {
    arrange({ ...READY, overridePath: '/usr/local/bin/claude' }, { api_cli_provider_set_path: READY });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Use default/ }));

    await waitFor(() => expect(calls('api_cli_provider_set_path')).toHaveLength(1));
    expect(calls('api_cli_provider_set_path')[0][1]).toEqual({ id: 'claude-code', path: null });
  });

  it('hides Use default when no location is saved, even with a found path', async () => {
    arrange(READY);
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);
    await screen.findByText(/\/usr\/local\/bin\/claude/);
    expect(screen.queryByRole('button', { name: /Use default/ })).toBeNull();
  });

  it('disables Locate while the file picker is open', async () => {
    let finishPick: (v: string | null) => void = () => {};
    arrange({ ...BASE, state: 'notFound' }, {
      api_cli_provider_pick_binary: () => new Promise((resolve) => { finishPick = resolve; }),
    });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    const locate = await screen.findByRole('button', { name: /Locate/ });
    fireEvent.click(locate);
    await waitFor(() => expect(locate).toBeDisabled());
    fireEvent.click(locate);
    expect(calls('api_cli_provider_pick_binary')).toHaveLength(1);

    finishPick(null);
    await waitFor(() => expect(locate).toBeEnabled());
  });

  it('names Claude Code as selected when the panel asks for status', async () => {
    arrange(READY);
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);
    await screen.findByText(/\/usr\/local\/bin\/claude/);
    expect(calls('api_cli_providers_status')[0][1]).toEqual({ selected: 'claude-code' });
  });

  it('the chooser dropdown asks for status without naming a selection', async () => {
    arrange(READY);
    render(
      <Select open value="ollama">
        <SelectTrigger><SelectValue /></SelectTrigger>
        <SelectContent>
          <CliProviderSelectItems />
        </SelectContent>
      </Select>,
    );
    await waitFor(() => expect(calls('api_cli_providers_status')).toHaveLength(1));
    expect(calls('api_cli_providers_status')[0][1]).toBeUndefined();
  });

  it('Send a test reports the result and says it uses a little of the plan', async () => {
    arrange(READY, { api_cli_provider_test: { ok: true, latencyMs: 1840 } });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Send a test/ }));
    expect(await screen.findByText(/Claude answered in 1\.8 s/)).toBeInTheDocument();
    expect(calls('api_cli_provider_test')[0][1]).toEqual({ id: 'claude-code', model: 'sonnet' });
    expect(screen.getByText(/uses a little of your plan/)).toBeInTheDocument();
  });

  it('Send a test shows the failure message', async () => {
    arrange(READY, { api_cli_provider_test: { ok: false, latencyMs: 90, error: 'Nixon could not reach Claude.' } });
    render(<CliProviderPanel provider="claude-code" onSave={vi.fn()} />);

    fireEvent.click(await screen.findByRole('button', { name: /Send a test/ }));
    expect(await screen.findByText('Nixon could not reach Claude.')).toBeInTheDocument();
  });

  it('asks for confirmation on the first Save only', async () => {
    arrange(READY);
    const onSave = vi.fn();
    const { unmount } = render(<CliProviderPanel provider="claude-code" onSave={onSave} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    // The confirm repeats the privacy line and says the tools are off.
    expect(await screen.findByText(/runs Claude Code with its tools turned off/)).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Use Claude Code' }));
    expect(onSave).toHaveBeenCalledTimes(1);
    unmount();

    // Second time: straight through.
    render(<CliProviderPanel provider="claude-code" onSave={onSave} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(onSave).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(/tools turned off/)).toBeNull();
  });

  it('cancelling the confirm saves nothing and asks again next time', async () => {
    arrange(READY);
    const onSave = vi.fn();
    render(<CliProviderPanel provider="claude-code" onSave={onSave} />);

    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText(/tools turned off/)).toBeInTheDocument();
  });

  it('lists Codex as coming soon, with nothing to save', async () => {
    arrange(READY);
    render(<CliProviderPanel provider="codex-cli" onSave={vi.fn()} />);
    expect(screen.getByText(/Coming soon/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
    expect(calls('api_cli_providers_status')).toHaveLength(0);
  });
});
