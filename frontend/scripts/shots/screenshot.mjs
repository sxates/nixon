// screenshot.mjs — headless-Chrome screenshot of a route, driven over the DevTools
// Protocol so we control timing (the dev server's HMR socket and the app's perpetual
// polling timers make the simpler `chrome --screenshot --virtual-time-budget` hang).
//
//   node screenshot.mjs <url> <out.png> [width] [height] [waitMs] [mockFile]
//
// Injects tauri-mock.js (sibling file by default) BEFORE the page boots so the app
// runs without the Tauri runtime. Writes <out.png> and a <out.png>.log breadcrumb.
//
// Thin wrapper over lib/cdp.mjs, which does the actual CDP work and is shared with
// the full-manifest driver (run.mjs, specs/0060).
import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { launchChrome, openSession, capture } from './lib/cdp.mjs';

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
const hardTimeout = setTimeout(() => { log('HARD_TIMEOUT'); process.exit(3); }, 28000);

let chrome;
try {
  chrome = await launchChrome();
  const browser = openSession(chrome.browserWs);
  await browser.ready;
  const png = await capture(browser, { url, mock: MOCK, width: +W, height: +H, waitMs: +WAIT });
  log('captured');
  writeFileSync(out, png);
  log('OK ' + png.length);
  browser.close();
} catch (e) {
  log('ERR ' + e.message);
} finally {
  clearTimeout(hardTimeout);
  chrome?.kill();
}
process.exit(0);
