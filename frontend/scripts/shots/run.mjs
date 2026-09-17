#!/usr/bin/env node
// run.mjs — capture every manifest entry headless (specs/0060).
//   node scripts/shots/run.mjs [--out DIR] [--only a,b] [--theme deck] [--no-build] [--worktree-build]
import { spawn, execSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadManifest, expand } from './lib/manifest.mjs';
import { launchChrome, openSession, capture } from './lib/cdp.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const frontend = resolve(here, '..', '..');
const repo = resolve(frontend, '..');

export function summarize(results) {
  const errors = results.filter((r) => r.status === 'error').length;
  return { ok: results.length - errors, errors, exitCode: errors ? 1 : 0 };
}

function args() {
  const a = process.argv.slice(2); const get = (k) => { const i = a.indexOf(k); return i >= 0 ? a[i + 1] : undefined; };
  return { out: get('--out') ?? join(repo, 'docs', 'screenshots', 'headless'), only: get('--only')?.split(','), theme: get('--theme'), noBuild: a.includes('--no-build'), worktreeBuild: a.includes('--worktree-build') };
}

function buildExport({ worktreeBuild }) {
  if (!worktreeBuild) { execSync('./node_modules/.bin/next build', { cwd: frontend, stdio: 'inherit' }); return join(frontend, 'out'); }
  const wt = execSync('mktemp -d').toString().trim() + '/wt';
  execSync(`git worktree add --detach "${wt}" HEAD`, { cwd: repo, stdio: 'inherit' });
  execSync(`ln -s "${frontend}/node_modules" "${wt}/frontend/node_modules"`);
  execSync('./node_modules/.bin/next build', { cwd: `${wt}/frontend`, stdio: 'inherit' });
  process.on('exit', () => { try { execSync(`git worktree remove --force "${wt}"`, { cwd: repo }); } catch {} });
  return `${wt}/frontend/out`;
}

function serve(root) {
  return new Promise((res, rej) => {
    const p = spawn(process.execPath, [join(here, 'serve-out.mjs'), root, '0'], { stdio: ['ignore', 'pipe', 'inherit'] });
    p.stdout.on('data', (d) => { const m = /:(\d+)\s*$/.exec(d.toString()); if (m) res({ port: +m[1], kill: () => p.kill() }); });
    p.on('exit', (c) => rej(new Error('serve-out exited ' + c)));
  });
}

async function main() {
  const opt = args();
  const devServerUp = (() => { try { execSync('lsof -i :3118 -sTCP:LISTEN', { stdio: 'ignore' }); return true; } catch { return false; } })();
  const root = opt.noBuild ? join(frontend, 'out') : buildExport({ worktreeBuild: opt.worktreeBuild || devServerUp });
  const server = await serve(root);
  const chrome = await launchChrome();
  const browser = openSession(chrome.browserWs); await browser.ready;
  const mock = readFileSync(join(here, 'tauri-mock.js'), 'utf8');
  mkdirSync(opt.out, { recursive: true });
  let shots = expand(loadManifest(), { headless: true });
  if (opt.only) shots = shots.filter((s) => opt.only.includes(s.name));
  if (opt.theme) shots = shots.filter((s) => s.theme === opt.theme);
  const results = [];
  for (const shot of shots) {
    const t0 = Date.now();
    try {
      const png = await capture(browser, { url: shot.url(`http://127.0.0.1:${server.port}`), mock, width: shot.entry.viewport[0], height: shot.entry.viewport[1], waitMs: shot.entry.wait });
      writeFileSync(join(opt.out, shot.file), png);
      results.push({ file: shot.file, status: 'ok', ms: Date.now() - t0 });
      console.log('ok  ', shot.file);
    } catch (e) {
      results.push({ file: shot.file, status: 'error', ms: Date.now() - t0, error: String(e.message || e) });
      console.error('ERR ', shot.file, e.message);
    }
  }
  const sha = (() => { try { return execSync('git rev-parse --short HEAD', { cwd: repo }).toString().trim(); } catch { return 'unknown'; } })();
  writeFileSync(join(opt.out, 'manifest-run.json'), JSON.stringify({ sha, at: new Date().toISOString(), shots: results }, null, 2));
  browser.close(); chrome.kill(); server.kill();
  const s = summarize(results);
  console.log(`${s.ok} ok, ${s.errors} errors`);
  process.exit(s.exitCode);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main().catch((e) => { console.error(e); process.exit(1); });
