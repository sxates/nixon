import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { AgentAccessSection } from '@/components/AgentAccessSection';

const off = {
  enabled: false,
  registered: false,
  serverName: 'nixon',
  manualCommand: 'claude mcp add nixon -- /Applications/Nixon.app/Contents/MacOS/nixon --mcp',
  lastError: null,
};

let log: unknown[] = [];
let setEnabledResult: unknown = { ...off, enabled: true, registered: true };

beforeEach(() => {
  log = [];
  setEnabledResult = { ...off, enabled: true, registered: true };
  invoke.mockReset();
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_agent_access_status') return off;
    if (cmd === 'api_agent_access_set_enabled') return setEnabledResult;
    if (cmd === 'api_agent_access_log') return log;
    if (cmd === 'api_agent_access_clear_log') {
      log = [];
      return undefined;
    }
    return undefined;
  });
});

const entry = (over: Record<string, unknown>) => ({
  at: '2026-11-01T15:04:00Z',
  client: 'claude-code',
  tool: 'search_meetings',
  args: { query: 'onboarding' },
  meetingIds: ['m1'],
  outcome: 'ok',
  meetings: [{ id: 'm1', title: 'Onboarding sync' }],
  ...over,
});

describe('AgentAccessSection', () => {
  it('renders_off_by_default', async () => {
    render(<AgentAccessSection />);
    expect(screen.getByText('Agent access')).toBeInTheDocument();
    const sw = await screen.findByRole('switch', { name: 'Let Claude Code read your meetings' });
    await waitFor(() => expect(sw).not.toBeDisabled());
    expect(sw).not.toBeChecked();
    expect(screen.getByText(/sent to Anthropic/)).toBeInTheDocument();
    expect(screen.getByText('Nothing has read your meetings yet.')).toBeInTheDocument();
  });

  it('toggle_on_calls_set_enabled_and_shows_registered', async () => {
    render(<AgentAccessSection />);
    const sw = await screen.findByRole('switch');
    await waitFor(() => expect(sw).not.toBeDisabled());
    fireEvent.click(sw);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('api_agent_access_set_enabled', { enabled: true }),
    );
    await waitFor(() => expect(screen.getByRole('switch')).toBeChecked());
    expect(screen.queryByText(/Run this in Terminal/)).not.toBeInTheDocument();
  });

  it('registration_failure_shows_manual_command_and_copy', async () => {
    setEnabledResult = { ...off, enabled: true, registered: false, lastError: 'boom' };
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<AgentAccessSection />);
    const sw = await screen.findByRole('switch');
    await waitFor(() => expect(sw).not.toBeDisabled());
    fireEvent.click(sw);
    expect(
      await screen.findByText(/Nixon couldn.t add itself to Claude Code\. Run this in Terminal:/),
    ).toBeInTheDocument();
    expect(screen.getByText(off.manualCommand)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(off.manualCommand));
  });

  it('log_rows_show_time_tool_terms_and_meeting_links', async () => {
    log = [
      entry({}),
      entry({
        tool: 'get_meeting',
        args: { meetingId: 'gone' },
        meetingIds: ['gone'],
        meetings: [{ id: 'gone', title: null }],
      }),
    ];
    render(<AgentAccessSection />);
    expect(await screen.findByText(/onboarding/, { selector: 'span' })).toBeInTheDocument();
    expect(screen.getByText('Searched')).toBeInTheDocument();
    expect(screen.getAllByText(/Nov 1, 2026/)).toHaveLength(2);
    expect(screen.getByRole('link', { name: 'Onboarding sync' })).toHaveAttribute(
      'href',
      '/meeting-details?id=m1',
    );
    const deleted = screen.getByText('Deleted meeting');
    expect(deleted.closest('a')).toBeNull();
  });

  it('refused_calls_are_labelled', async () => {
    log = [entry({ outcome: 'off', meetingIds: [], meetings: [] })];
    render(<AgentAccessSection />);
    expect(await screen.findByText('Refused — access was off')).toBeInTheDocument();
  });

  it('clear_log_empties_list', async () => {
    log = [entry({})];
    render(<AgentAccessSection />);
    fireEvent.click(await screen.findByRole('button', { name: 'Clear log' }));
    expect(await screen.findByText('Nothing has read your meetings yet.')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('api_agent_access_clear_log');
  });
});
