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
  desktopInstalled: true,
  desktopRegistered: false,
  desktopError: null,
};

let log: unknown[] = [];
let setEnabledResult: unknown = { ...off, enabled: true, registered: true };

beforeEach(() => {
  log = [];
  setEnabledResult = { ...off, enabled: true, registered: true, desktopRegistered: true };
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
    const sw = await screen.findByRole('switch', { name: 'Let Claude read your meetings' });
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
      await screen.findByText(/Nixon couldn.t add itself to Claude Code\. If you haven.t already, run this in Terminal:/),
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

  it('log_meeting_link_encodes_the_id', async () => {
    log = [entry({ meetingIds: ['a&b=c'], meetings: [{ id: 'a&b=c', title: 'Odd id' }] })];
    render(<AgentAccessSection />);
    expect(await screen.findByRole('link', { name: 'Odd id' })).toHaveAttribute(
      'href',
      '/meeting-details?id=a%26b%3Dc',
    );
  });

  it('log_rows_show_real_snake_case_filters_for_each_tool', async () => {
    log = [
      entry({ args: { query: 'budget', from_date: '2026-10-01', to_date: '2026-10-31', limit: 5 } }),
      entry({
        tool: 'list_meetings',
        args: { person: 'dana', title_contains: 'standup', from_date: '2026-09-01' },
      }),
      entry({
        tool: 'get_meeting',
        args: { meeting_id: 'm1', from_seconds: 120, to_seconds: 300.5, cursor: 2 },
      }),
    ];
    render(<AgentAccessSection />);
    expect(await screen.findByText(/budget/)).toHaveTextContent(
      '\u201Cbudget\u201D \u00B7 2026-10-01 \u2192 2026-10-31',
    );
    expect(screen.getByText(/dana/)).toHaveTextContent(
      'title \u201Cstandup\u201D \u00B7 dana \u00B7 from 2026-09-01',
    );
    expect(screen.getByText(/120s/)).toHaveTextContent('120s \u2192 300.5s');
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

  it('asks_only_for_the_newest_ten_rows_and_offers_the_full_log', async () => {
    log = [entry({})];
    render(<AgentAccessSection />);
    fireEvent.click(await screen.findByRole('button', { name: 'Open full log' }));
    expect(invoke).toHaveBeenCalledWith('api_agent_access_log', { limit: 10 });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_agent_access_open_full_log'));
  });

  it('no_full_log_link_when_the_log_is_empty', async () => {
    render(<AgentAccessSection />);
    await screen.findByText('Nothing has read your meetings yet.');
    expect(screen.queryByRole('button', { name: 'Open full log' })).not.toBeInTheDocument();
  });

  it('toggling_tells_you_to_restart_claude_desktop', async () => {
    render(<AgentAccessSection />);
    const sw = await screen.findByRole('switch');
    await waitFor(() => expect(sw).not.toBeDisabled());
    expect(screen.queryByText(/quit and reopen it/)).not.toBeInTheDocument();
    fireEvent.click(sw);
    expect(await screen.findByText(/quit and reopen it/)).toBeInTheDocument();
  });

  it('no_restart_note_without_claude_desktop', async () => {
    setEnabledResult = { ...off, enabled: true, registered: true, desktopInstalled: false };
    render(<AgentAccessSection />);
    const sw = await screen.findByRole('switch');
    await waitFor(() => expect(sw).not.toBeDisabled());
    fireEvent.click(sw);
    await waitFor(() => expect(screen.getByRole('switch')).toBeChecked());
    expect(screen.queryByText(/quit and reopen it/)).not.toBeInTheDocument();
  });

  it('desktop_failure_is_shown_without_blocking_claude_code', async () => {
    setEnabledResult = {
      ...off,
      enabled: true,
      registered: true,
      desktopError: 'Claude’s settings file isn’t in a format Nixon understands.',
    };
    render(<AgentAccessSection />);
    const sw = await screen.findByRole('switch');
    await waitFor(() => expect(sw).not.toBeDisabled());
    fireEvent.click(sw);
    expect(
      await screen.findByText(/Nixon couldn.t add itself to Claude desktop chats\. Claude.s settings file/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/run this in Terminal/)).not.toBeInTheDocument();
    expect(screen.queryByText(/quit and reopen it/)).not.toBeInTheDocument();
  });
});
