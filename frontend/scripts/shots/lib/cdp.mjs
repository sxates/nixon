// cdp.mjs — one headless Chrome, many captures, over the DevTools Protocol (specs/0060).
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { rmSync } from 'node:fs';

export const DEFAULT_CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function readyPollScript() {
  return "document.documentElement.dataset.shotReady === '1'";
}

/** Race `promise` against a `ms`-timeout, rejecting with a message naming `label` (the
 *  shot's URL) if it fires first. Always clears its own timer, on either outcome, so a
 *  wedged CDP target fails fast instead of relying on each RPC's own 15s timeout to add
 *  up past the run's time budget (specs/0060 Task 3 review). */
export function withDeadline(promise, ms, label) {
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`shot deadline ${ms} ms exceeded: ${label}`)), ms);
  });
  return Promise.race([promise, deadline]).finally(() => clearTimeout(timer));
}

export async function launchChrome({ chromePath = process.env.CHROME_PATH || DEFAULT_CHROME } = {}) {
  const port = 9400 + Math.floor(Math.random() * 400);
  const profile = join(tmpdir(), `nixon-shots-${process.pid}`);
  const proc = spawn(chromePath, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--no-first-run', '--no-default-browser-check',
    '--disable-extensions', '--hide-scrollbars', '--force-color-profile=srgb', '--font-render-hinting=none',
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, 'about:blank',
  ], { stdio: 'ignore' });
  let browserWs;
  for (let i = 0; i < 100 && !browserWs; i++) {
    try { browserWs = (await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()).webSocketDebuggerUrl; } catch { await sleep(100); }
  }
  if (!browserWs) { proc.kill('SIGKILL'); throw new Error('chrome did not expose CDP'); }
  return { port, browserWs, kill() { proc.kill('SIGKILL'); rmSync(profile, { recursive: true, force: true }); } };
}

export function openSession(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0; const pending = new Map(); const listeners = new Map();
  const send = (method, params = {}, t = 15000, sessionId) => new Promise((res, rej) => {
    const mid = ++id;
    const to = setTimeout(() => { pending.delete(mid); rej(new Error('timeout ' + method)); }, t);
    pending.set(mid, { res, rej, to });
    ws.send(JSON.stringify({ id: mid, method, params, sessionId }));
  });
  ws.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { const p = pending.get(m.id); clearTimeout(p.to); pending.delete(m.id); m.error ? p.rej(new Error(m.error.message)) : p.res(m.result); }
    else if (m.method && listeners.has(m.method)) listeners.get(m.method)(m.params);
  });
  const opened = new Promise((r) => ws.addEventListener('open', r, { once: true }));
  return { ready: opened, send, on: (m, f) => listeners.set(m, f), close: () => ws.close() };
}

/** Open a fresh target, inject the mock, navigate, wait for readiness, capture, close the
 *  target. The whole body races against `deadlineMs` (default 30s) — each CDP RPC already
 *  has its own 15s timeout, but a target that keeps answering slowly (rather than timing
 *  out outright) could otherwise take minutes to fail and blow the run's time budget. */
export async function capture(browser, { url, mock, width, height, waitMs, readyTimeoutMs = 10000, deadlineMs = 30000 }) {
  let targetId;
  const body = (async () => {
    ({ targetId } = await browser.send('Target.createTarget', { url: 'about:blank' }));
    const { sessionId } = await browser.send('Target.attachToTarget', { targetId, flatten: true });
    const s = (m, p, t) => browser.send(m, p, t, sessionId);
    await s('Page.enable'); await s('Runtime.enable');
    if (mock) await s('Page.addScriptToEvaluateOnNewDocument', { source: mock });
    await s('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
    await s('Page.navigate', { url });
    const t0 = Date.now(); let ready = false;
    while (Date.now() - t0 < readyTimeoutMs) {
      const r = await s('Runtime.evaluate', { expression: readyPollScript(), returnByValue: true });
      if (r?.result?.value === true) { ready = true; break; }
      await sleep(100);
    }
    if (!ready) throw new Error(`not ready after ${readyTimeoutMs} ms: ${url}`);
    await sleep(waitMs);
    const shot = await s('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    return Buffer.from(shot.data, 'base64');
  })();
  try {
    return await withDeadline(body, deadlineMs, url);
  } finally {
    if (targetId) await browser.send('Target.closeTarget', { targetId }).catch(() => {});
  }
}
