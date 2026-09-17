'use client';

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

// specs/0058 — one subscription to the backend's `update-status` event, shared by the
// sidebar row, Settings > About and anything the tray triggers. Mirrors the Rust enum
// (serde tag = "state", kebab-case).
export type UpdateStatus =
  | { state: 'idle'; last_checked: string | null }
  | { state: 'checking' }
  | { state: 'downloading'; version: string; received: number; total: number | null }
  | { state: 'ready'; version: string; notes: string; last_checked: string | null }
  | { state: 'error'; message: string; last_checked: string };

const STATES = new Set(['idle', 'checking', 'downloading', 'ready', 'error']);
const IDLE: UpdateStatus = { state: 'idle', last_checked: null };

export function parseUpdateStatus(payload: unknown): UpdateStatus | null {
  if (!payload || typeof payload !== 'object') return null;
  const s = (payload as { state?: unknown }).state;
  return typeof s === 'string' && STATES.has(s) ? (payload as UpdateStatus) : null;
}

function relative(iso: string, now: Date): string {
  const secs = Math.max(0, Math.round((now.getTime() - new Date(iso).getTime()) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

/** The About panel's one-line status. */
export function describeStatus(status: UpdateStatus, now: Date = new Date()): string {
  switch (status.state) {
    case 'idle':
      return status.last_checked ? `Up to date · checked ${relative(status.last_checked, now)}` : 'Not checked yet';
    case 'checking':
      return 'Checking…';
    case 'downloading':
      return `Downloading ${status.version}…`;
    case 'ready': {
      const line = `${status.version} ready — restart to update`;
      return status.last_checked ? `${line} · checked ${relative(status.last_checked, now)}` : line;
    }
    case 'error':
      return "Couldn't check for updates";
  }
}

interface UpdateStatusValue {
  status: UpdateStatus;
  busy: boolean;
  error: string | null;
  checkNow: () => Promise<void>;
  install: () => Promise<void>;
}

const Ctx = createContext<UpdateStatusValue | null>(null);

export function UpdateStatusProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<UpdateStatus>(IDLE);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    invoke<unknown>('api_get_update_status')
      .then((s) => { const p = parseUpdateStatus(s); if (p && !disposed) setStatus(p); })
      .catch(() => { /* not in Tauri (tests, plain browser) — stay idle */ });
    const unlisten = safeListen<unknown>('update-status', (e) => {
      const p = parseUpdateStatus(e.payload);
      // A fresh status supersedes whatever the last failure said — otherwise a refusal
      // from days ago sits under the row forever.
      if (p) { setStatus(p); setError(null); }
    });
    // specs/0058 — an install can be refused from the tray, where there is no caller to
    // return the message to. The backend broadcasts every refusal so the UI shows it
    // wherever the user happens to be looking.
    const unlistenRefused = safeListen<unknown>('update-install-refused', (e) => {
      const message = (e.payload as { message?: unknown } | null)?.message;
      if (typeof message === 'string' && message) setError(message);
    });
    return () => { disposed = true; unlisten(); unlistenRefused(); };
  }, []);

  const checkNow = useCallback(async () => {
    setBusy(true); setError(null);
    try {
      const p = parseUpdateStatus(await invoke<unknown>('api_check_for_updates'));
      if (p) setStatus(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const install = useCallback(async () => {
    setBusy(true); setError(null);
    try {
      await invoke('api_install_update'); // on success the app restarts; nothing to do
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }, []);

  const value = useMemo(() => ({ status, busy, error, checkNow, install }), [status, busy, error, checkNow, install]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useUpdateStatus(): UpdateStatusValue {
  const v = useContext(Ctx);
  if (!v) throw new Error('useUpdateStatus must be used within an UpdateStatusProvider');
  return v;
}

/** For chrome mounted app-wide (the sidebar): tolerate a tree without the provider. */
export function useOptionalUpdateStatus(): UpdateStatusValue | null {
  return useContext(Ctx);
}
