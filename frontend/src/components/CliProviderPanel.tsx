'use client';

/**
 * Settings panel for the subscription CLI providers (specs/0085).
 *
 * Claude Code runs under the user's own login, so there is no key to enter and no sign-in
 * flow here: Nixon checks that it can reach the service and says so in plain words. Save is
 * only possible when the check passes. The Rust side re-probes on every status call, so
 * "Check again" is just asking again.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import * as SelectPrimitive from '@radix-ui/react-select';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { SelectGroup, SelectLabel } from '@/components/ui/select';
import { Spinner } from '@/components/ui/spinner';
import { StatusDot } from '@/components/ui/status-dot';
import type { CliProviderId } from '@/lib/llm-providers';

type CliState = 'notFound' | 'tooOld' | 'needsSignIn' | 'ready';

export interface CliProviderStatus {
  id: CliProviderId;
  displayName: string;
  state: CliState;
  path?: string;
  version?: string;
  minVersion: string;
  authNote?: string;
  probeError?: string;
  /** Set only when a Locate… path is saved. */
  overridePath?: string;
  models: { id: string; label: string }[];
}

interface CliTestResult {
  ok: boolean;
  latencyMs: number;
  error?: string;
}

const INSTALL_URL = 'https://docs.claude.com/en/docs/claude-code/overview';
const CONFIRMED_KEY = (id: string) => `nixon.cliProviderConfirmed.${id}`;

const PRIVACY =
  "Meeting transcripts, notes and participant names are sent to Anthropic through Claude Code, under your Claude plan's terms and privacy settings. Summaries, action items, Ask AI and prep briefs all use it.";
const PRIVACY_EXTRA =
  'Claude Code also sends your account email and basic details about your Mac with each request.';
const AUTH_COPY =
  "Nixon couldn't reach Claude with your account. Make sure you're signed in to Claude Code on this Mac, then Check again.";

function readConfirmed(id: string): boolean {
  try {
    return localStorage.getItem(CONFIRMED_KEY(id)) === '1';
  } catch {
    return false;
  }
}

function writeConfirmed(id: string) {
  try {
    localStorage.setItem(CONFIRMED_KEY(id), '1');
  } catch {
    /* private window: ask again next time */
  }
}

const errText = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong.');

/**
 * Fetch the CLI providers' status; `refresh` re-probes. `selected` is the CLI whose panel is
 * showing: it lets the Rust side try the login shell for that CLI. The dropdown passes none.
 */
function useCliStatuses(selected?: CliProviderId) {
  const [statuses, setStatuses] = useState<CliProviderStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);

  const refresh = useCallback(async () => {
    setChecking(true);
    try {
      setStatuses(
        await (selected
          ? invoke<CliProviderStatus[]>('api_cli_providers_status', { selected })
          : invoke<CliProviderStatus[]>('api_cli_providers_status')),
      );
      setError(null);
    } catch (e) {
      setError(errText(e));
    } finally {
      setChecking(false);
    }
  }, [selected]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { statuses, setStatuses, error, checking, refresh };
}

const STATE_WORD: Record<CliState, string> = {
  ready: 'Ready',
  needsSignIn: 'Needs sign-in',
  notFound: 'Not found',
  tooOld: 'Needs an update',
};

/**
 * The "Your AI subscription" group of the provider chooser. Radix mounts the items only
 * while the list is open, so the probe runs when the user looks, not on every settings load.
 * The state word sits outside `ItemText`, so the closed trigger still just reads the name.
 */
export function CliProviderSelectItems() {
  const { statuses } = useCliStatuses();
  const claude = statuses?.find((s) => s.id === 'claude-code');
  const itemClass =
    'relative flex w-full cursor-default select-none items-center justify-between gap-3 rounded-sm py-1.5 pl-2 pr-8 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50';
  return (
    <SelectGroup>
      <SelectLabel>Your AI subscription</SelectLabel>
      <SelectPrimitive.Item value="claude-code" className={itemClass}>
        <SelectPrimitive.ItemText>Claude Code</SelectPrimitive.ItemText>
        <span className="text-xs text-muted-foreground">{claude ? STATE_WORD[claude.state] : ''}</span>
      </SelectPrimitive.Item>
      <SelectPrimitive.Item value="codex-cli" disabled className={itemClass}>
        <SelectPrimitive.ItemText>Codex</SelectPrimitive.ItemText>
        <span className="text-xs text-muted-foreground">Coming soon</span>
      </SelectPrimitive.Item>
    </SelectGroup>
  );
}

export interface CliProviderPanelProps {
  provider: CliProviderId;
  /** Called once the user has saved (after the one-time confirm). */
  onSave: () => void;
}

export function CliProviderPanel({ provider, onSave }: CliProviderPanelProps) {
  if (provider !== 'claude-code') {
    return (
      <p className="mt-4 text-sm text-muted-foreground">
        Codex · Coming soon. Nixon will be able to use your ChatGPT plan through Codex.
      </p>
    );
  }
  return <ClaudeCodePanel onSave={onSave} />;
}

function StatusLine({ status }: { status: CliProviderStatus }) {
  const { state, probeError } = status;
  // Status is a dot, not a glyph: green when Claude is reachable, destructive otherwise.
  const ok = !probeError && state === 'ready';
  let text: string;
  if (probeError) {
    text = probeError;
  } else if (state === 'ready') {
    text = 'Ready. Nixon can reach Claude with your account.';
  } else if (state === 'needsSignIn') {
    text = AUTH_COPY;
  } else if (state === 'tooOld') {
    text = `Update Claude Code, then Check again. Nixon needs version ${status.minVersion} or newer.`;
  } else {
    text = "Nixon couldn't find Claude Code on this Mac. Install it, or choose where it lives with Locate…";
  }
  return (
    <div className="flex items-start gap-2 text-sm" data-ok={ok}>
      <StatusDot tone={ok ? 'success' : 'destructive'} className="mt-1.5" />
      <span>{text}</span>
    </div>
  );
}

function ClaudeCodePanel({ onSave }: { onSave: () => void }) {
  const { statuses, setStatuses, error, checking, refresh } = useCliStatuses('claude-code');
  const status = statuses?.find((s) => s.id === 'claude-code') ?? null;

  const [pathError, setPathError] = useState<string | null>(null);
  const [applyingPath, setApplyingPath] = useState(false);
  const [picking, setPicking] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<CliTestResult | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const ready = status?.state === 'ready' && !status.probeError;
  const showInstall = status?.state === 'notFound' && !status.probeError;

  const applyPath = async (path: string | null) => {
    setApplyingPath(true);
    setPathError(null);
    try {
      const next = await invoke<CliProviderStatus>('api_cli_provider_set_path', { id: 'claude-code', path });
      setStatuses([next]);
    } catch (e) {
      setPathError(errText(e));
    } finally {
      setApplyingPath(false);
    }
  };

  // Locate…: the native file chooser picks, set_path validates and saves. A cancel is not an error.
  const locate = async () => {
    if (picking) return;
    setPathError(null);
    setPicking(true);
    let picked: string | null = null;
    try {
      picked = await invoke<string | null>('api_cli_provider_pick_binary');
    } catch (e) {
      setPathError(errText(e));
    } finally {
      setPicking(false);
    }
    if (picked) await applyPath(picked);
  };

  const sendTest = async () => {
    setTesting(true);
    setTestResult(null);
    setTestError(null);
    try {
      setTestResult(await invoke<CliTestResult>('api_cli_provider_test', { id: 'claude-code', model: 'sonnet' }));
    } catch (e) {
      setTestError(errText(e));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = () => {
    if (readConfirmed('claude-code')) onSave();
    else setConfirmOpen(true);
  };

  const confirm = () => {
    writeConfirmed('claude-code');
    setConfirmOpen(false);
    onSave();
  };

  return (
    <div className="mt-4 space-y-4" data-testid="cli-provider-panel">
      <div className="space-y-2 rounded-md border border-border bg-card p-3">
        {status ? (
          <>
            <StatusLine status={status} />
            {(status.path || status.version) && (
              <p className="break-all text-xs text-muted-foreground">
                {status.path}
                {status.path && status.version ? ' · ' : ''}
                {status.version ? `version ${status.version}` : ''}
              </p>
            )}
            {status.authNote && <p className="text-xs text-muted-foreground">{status.authNote}</p>}
          </>
        ) : (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            {error ? error : (
              <>
                <Spinner /> Checking Claude Code…
              </>
            )}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2 pt-1">
          <Button type="button" variant="outline" size="sm" onClick={() => void refresh()} disabled={checking}>
            {checking ? 'Checking…' : 'Check again'}
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => void locate()}
            disabled={applyingPath || picking}>
            Locate…
          </Button>
          {showInstall && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => invoke('open_external_url', { url: INSTALL_URL })}
            >
              How to install
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          Signed in somewhere else? Choose the program you use with Locate…
        </p>
        {status?.overridePath && (
          <Button
            type="button"
            variant="link"
            size="xs"
            className="h-auto px-0 text-muted-foreground underline"
            onClick={() => void applyPath(null)}
            disabled={applyingPath}
          >
            Use default
          </Button>
        )}
        {pathError && <p className="text-sm text-destructive">{pathError}</p>}
      </div>

      <p className="text-sm">Model: Sonnet</p>

      <div className="space-y-1 text-sm text-muted-foreground">
        <p>{PRIVACY}</p>
        <p>{PRIVACY_EXTRA}</p>
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button type="button" variant="outline" size="sm" onClick={() => void sendTest()} disabled={!ready || testing}>
          {testing ? 'Testing…' : 'Send a test'}
        </Button>
        <span className="text-xs text-muted-foreground">Sending a test uses a little of your plan.</span>
      </div>
      {testResult?.ok && (
        <p className="text-sm text-success">Claude answered in {(testResult.latencyMs / 1000).toFixed(1)} s.</p>
      )}
      {testResult && !testResult.ok && <p className="text-sm text-destructive">{testResult.error ?? AUTH_COPY}</p>}
      {testError && <p className="text-sm text-destructive">{testError}</p>}

      <div className="flex justify-end">
        <Button
          type="button"
          variant="brand"
          disabled={!ready}
          onClick={handleSave}
        >
          Save
        </Button>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title="Use Claude Code for your meetings?"
        body={PRIVACY}
        tone="brand"
        confirmLabel="Use Claude Code"
        onConfirm={confirm}
      >
        <p className="text-sm text-muted-foreground">{PRIVACY_EXTRA}</p>
        <p className="text-sm">
          Nixon runs Claude Code with its tools turned off. It can&apos;t read or change files on your Mac.
        </p>
      </ConfirmDialog>
    </div>
  );
}
