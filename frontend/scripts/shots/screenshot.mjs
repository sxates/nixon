// screenshot.mjs — headless-Chrome screenshot of a route, driven over the DevTools
// Protocol so we control timing (the dev server's HMR socket and the app's perpetual
// polling timers make the simpler `chrome --screenshot --virtual-time-budget` hang).
//
//   node screenshot.mjs <url> <out.png> [width] [height] [waitMs] [mockFile]
//
// Injects tauri-mock.js (sibling file by default) BEFORE the page boots so the app
// runs without the Tauri runtime. Writes <out.png> and a <out.png>.log breadcrumb.
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const [url, out, W = '1280', H = '860', WAIT = '5000', mockFile = join(here, 'tauri-mock.js')] =
  process.argv.slice(2);
if (!url || !out) {
  console.error('usage: node screenshot.mjs <url> <out.png> [w] [h] [waitMs] [mockFile]');
  process.exit(2);
}
const MOCK = readFileSync(mockFile, 'utf8');
const LOG = out + '.log';
const log = (m) => { try { appendFileSync(LOG, m + '\n'); } catch {} };
writeFileSync(LOG, '');
setTimeout(() => { log('HARD_TIMEOUT'); process.exit(3); }, 28000);

const CHROME = process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const PORT = 9400 + Math.floor(Math.random() * 400);
const PROFILE = join(tmpdir(), `nixon-shot-${process.pid}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const chrome = spawn(CHROME, [
  '--headless=new', '--disable-gpu', '--no-sandbox', '--no-first-run',
  '--no-default-browser-check', '--disable-extensions', '--hide-scrollbars',
  `--remote-debugging-port=${PORT}`, `--user-data-dir=${PROFILE}`, 'about:blank',
], { stdio: 'ignore' });

let wsUrl;
for (let i = 0; i < 60; i++) {
  try {
    const r = await fetch(`http://127.0.0.1:${PORT}/json`);
    const t = await r.json();
    const pg = t.find((x) => x.type === 'page');
    if (pg) { wsUrl = pg.webSocketDebuggerUrl; break; }
  } catch {}
  await sleep(200);
}
if (!wsUrl) { log('NO_WS'); chrome.kill('SIGKILL'); process.exit(1); }

const ws = new WebSocket(wsUrl);
let id = 0;
const pending = new Map();
const send = (method, params = {}, t = 15000) =>
  new Promise((res, rej) => {
    const mid = ++id;
    const to = setTimeout(() => { pending.delete(mid); rej(new Error('timeout ' + method)); }, t);
    pending.set(mid, (r) => { clearTimeout(to); res(r); });
    ws.send(JSON.stringify({ id: mid, method, params }));
  });
await new Promise((r) => ws.addEventListener('open', r, { once: true }));
ws.addEventListener('message', (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) pending.get(m.id)(m.result);
});

try {
  await send('Page.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: MOCK });
  await send('Emulation.setDeviceMetricsOverride', { width: +W, height: +H, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url });
  log('navigated');
  await sleep(+WAIT);
  const r = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false, optimizeForSpeed: true });
  if (r && r.data) { writeFileSync(out, Buffer.from(r.data, 'base64')); log('OK ' + r.data.length); }
  else log('NO_DATA');
} catch (e) {
  log('ERR ' + e.message);
}
ws.close();
chrome.kill('SIGKILL');
process.exit(0);
