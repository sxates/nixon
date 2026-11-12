'use client';

/**
 * Settings > General > Agent access (specs/0087). Off by default. When on, Nixon registers
 * itself as a read-only MCP server with Claude Code and, when it is installed, the Claude
 * desktop app's chats (Amendment A). The newest calls are listed below; the full 90-day log
 * opens as a text file.
 */

import { useCallback, useEffect, useState } from 'react';
import Link from 'next/link';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

interface AgentAccessStatus {
  enabled: boolean;
  registered: boolean;
  serverName: string;
  manualCommand: string;
  lastError: string | null;
  desktopInstalled: boolean;
  desktopRegistered: boolean;
  desktopError: string | null;
}

/** Settings shows only the newest calls; the rest are in the full log file. */
const RECENT_LIMIT = 10;

interface AccessEntry {
  at: string;
  client: string | null;
  tool: string;
  args: Record<string, unknown> | null;
  meetingIds: string[];
  outcome: 'ok' | 'off' | 'needs_update' | 'no_db' | 'error' | string;
  meetings: { id: string; title: string | null }[];
}

const TOOL_LABELS: Record<string, string> = {
  search_meetings: 'Searched',
  list_meetings: 'Listed meetings',
  get_meeting: 'Read a meeting',
};

const REFUSALS: Record<string, string> = {
  off: 'Refused — access was off',
  needs_update: 'Refused — Nixon needed updating',
  no_db: 'Refused — no meetings yet',
  error: 'Error',
};

/** The search terms and filters, compactly. Keys are the snake_case MCP tool args (queries.rs). */
function describeArgs(args: AccessEntry['args']): string {
  if (!args || typeof args !== 'object') return '';
  const parts: string[] = [];
  const str = (v: unknown) => (typeof v === 'string' && v.trim() ? v.trim() : null);
  const query = str(args.query);
  if (query) parts.push(`“${query}”`);
  const title = str(args.title_contains);
  if (title) parts.push(`title “${title}”`);
  const person = str(args.person);
  if (person) parts.push(person);
  const from = str(args.from_date);
  const to = str(args.to_date);
  if (from && to) parts.push(`${from} → ${to}`);
  else if (from) parts.push(`from ${from}`);
  else if (to) parts.push(`until ${to}`);
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  const fromS = num(args.from_seconds);
  const toS = num(args.to_seconds);
  if (fromS !== null && toS !== null) parts.push(`${fromS}s \u2192 ${toS}s`);
  else if (fromS !== null) parts.push(`from ${fromS}s`);
  else if (toS !== null) parts.push(`until ${toS}s`);
  return parts.join(' · ');
}

function formatTime(at: string): string {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  return d.toLocaleString('en-US', { dateStyle: 'medium', timeStyle: 'short' });
}

export function AgentAccessSection() {
  const [status, setStatus] = useState<AgentAccessStatus | null>(null);
  const [log, setLog] = useState<AccessEntry[]>([]);
  const [busy, setBusy] = useState(false);
  // Claude desktop reads its MCP list at launch, so a change made here needs a restart.
  const [desktopChanged, setDesktopChanged] = useState(false);

  const loadLog = useCallback(async () => {
    try {
      const rows = await invoke<AccessEntry[]>('api_agent_access_log', { limit: RECENT_LIMIT });
      setLog(Array.isArray(rows) ? rows : []);
    } catch (error) {
      console.error('Failed to load agent access log:', error);
    }
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        setStatus(await invoke<AgentAccessStatus>('api_agent_access_status'));
      } catch (error) {
        console.error('Failed to load agent access status:', error);
      }
    })();
    void loadLog();
    // Claude Code calls happen while Settings is open in another window; refresh on return.
    const onFocus = () => void loadLog();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [loadLog]);

  const onToggle = async (next: boolean) => {
    setBusy(true);
    try {
      const st = await invoke<AgentAccessStatus>('api_agent_access_set_enabled', { enabled: next });
      setStatus(st);
      setDesktopChanged(!!st?.desktopInstalled && !st?.desktopError);
    } catch (error) {
      console.error('Failed to change agent access:', error);
      toast.error('Failed to change agent access');
    } finally {
      setBusy(false);
    }
  };

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(status?.manualCommand ?? '');
      toast.success('Copied');
    } catch {
      toast.error('Could not copy');
    }
  };

  const onOpenFullLog = async () => {
    try {
      await invoke('api_agent_access_open_full_log');
    } catch (error) {
      console.error('Failed to open the full agent access log:', error);
      toast.error('Could not open the full log');
    }
  };

  const onClear = async () => {
    try {
      await invoke('api_agent_access_clear_log');
      setLog([]);
    } catch (error) {
      console.error('Failed to clear agent access log:', error);
      toast.error('Failed to clear the log');
    }
  };

  const enabled = status?.enabled ?? false;
  const failed = enabled && !status?.registered;

  return (
    <SettingsSection id="agent-access" title="Agent access">
      <SettingsGroup>
        <SettingsRow
          label="Let Claude read your meetings"
          description="Claude Code and the Claude desktop app can search and read all of your meetings — transcripts, summaries, notes and action items. It can't change anything. Whatever Claude reads becomes part of your Claude conversation and is sent to Anthropic, like anything else you show Claude."
          control={
            <Switch
              checked={enabled}
              disabled={busy || !status}
              onCheckedChange={onToggle}
              aria-label="Let Claude read your meetings"
            />
          }
        >
          {failed ? (
            <div className="space-y-2">
              <p className="text-sm text-foreground">
                Nixon couldn&rsquo;t add itself to Claude Code. If you haven&rsquo;t already, run this in Terminal:
              </p>
              <div className="flex items-start gap-2">
                <code className="min-w-0 flex-1 break-all rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs text-foreground">
                  {status?.manualCommand}
                </code>
                <Button variant="outline" size="sm" onClick={onCopy}>
                  Copy
                </Button>
              </div>
              {status?.lastError ? (
                <p className="text-xs text-muted-foreground">{status.lastError}</p>
              ) : null}
            </div>
          ) : null}
          {status?.desktopError ? (
            <p className="text-sm text-foreground">
              Nixon couldn&rsquo;t add itself to Claude desktop chats. {status.desktopError}
            </p>
          ) : desktopChanged ? (
            <p className="text-sm text-muted-foreground">
              If the Claude desktop app is open, quit and reopen it so its chats pick up the change.
            </p>
          ) : null}
        </SettingsRow>
        <div className="py-3">
          <div className="flex items-center justify-between gap-4">
            <h3 className="text-sm font-medium text-foreground">Recent access</h3>
            {log.length > 0 ? (
              <Button variant="outline" size="sm" onClick={onClear}>
                Clear log
              </Button>
            ) : null}
          </div>
          {log.length === 0 ? (
            <p className="mt-2 text-sm text-muted-foreground">
              Nothing has read your meetings yet.
            </p>
          ) : (
            <ul className="mt-2 divide-y divide-border">
              {log.map((e, i) => {
                const terms = describeArgs(e.args);
                const refusal = e.outcome !== 'ok' ? (REFUSALS[e.outcome] ?? 'Error') : null;
                return (
                  <li key={`${e.at}-${i}`} className="py-2 text-sm">
                    <div className="flex flex-wrap items-baseline gap-x-2">
                      <span className="text-muted-foreground">{formatTime(e.at)}</span>
                      <span className="font-medium text-foreground">
                        {TOOL_LABELS[e.tool] ?? e.tool}
                      </span>
                      {terms ? <span className="text-foreground">{terms}</span> : null}
                      {refusal ? <span className="text-muted-foreground">{refusal}</span> : null}
                    </div>
                    {e.meetings?.length ? (
                      <div className="mt-0.5 flex flex-wrap gap-x-3 text-xs">
                        {e.meetings.map((m) =>
                          m.title ? (
                            <Link
                              key={m.id}
                              href={`/meeting-details?id=${encodeURIComponent(m.id)}`}
                              className="text-primary hover:underline"
                            >
                              {m.title}
                            </Link>
                          ) : (
                            <span key={m.id} className="text-muted-foreground">
                              Deleted meeting
                            </span>
                          ),
                        )}
                      </div>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
          {log.length > 0 ? (
            <Button variant="link" size="xs" onClick={onOpenFullLog} className="mt-2 h-auto px-0 text-sm">
              Open full log
            </Button>
          ) : null}
        </div>
      </SettingsGroup>
    </SettingsSection>
  );
}
