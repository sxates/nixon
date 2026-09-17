#!/usr/bin/env node
// diff.mjs — compare the headless set against a git ref and write a contact sheet (specs/0060).
import { execFileSync } from 'node:child_process';
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { PNG } from 'pngjs';
import pixelmatch from 'pixelmatch';
import { loadManifest } from './lib/manifest.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..', '..');

export function diffPng(aBuf, bBuf, { threshold = 0.1, ignore = [] } = {}) {
  const a = PNG.sync.read(aBuf), b = PNG.sync.read(bBuf);
  if (a.width !== b.width || a.height !== b.height) throw new Error(`size mismatch ${a.width}x${a.height} vs ${b.width}x${b.height}`);
  for (const [x, y, w, h] of ignore) {
    const x0 = Math.max(0, x), y0 = Math.max(0, y);
    const x1 = Math.min(a.width, x + w), y1 = Math.min(a.height, y + h);
    for (let yy = y0; yy < y1; yy++) for (let xx = x0; xx < x1; xx++) { const i = (yy * a.width + xx) * 4; b.data.set(a.data.subarray(i, i + 4), i); }
  }
  const out = new PNG({ width: a.width, height: a.height });
  const changed = pixelmatch(a.data, b.data, out.data, a.width, a.height, { threshold, includeAA: false });
  return { changed, total: a.width * a.height, ratio: changed / (a.width * a.height), diffPng: PNG.sync.write(out) };
}

function fromGit(ref, relPath) {
  try { return execFileSync('git', ['show', `${ref}:${relPath}`], { cwd: repo, stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 64 * 1024 * 1024 }); } catch { return null; }
}

// routes.json ignore rects are {x,y,w,h,why} objects (see manifest.mjs); diffPng wants [x,y,w,h] tuples.
const toRects = (ignore) => (ignore ?? []).map((r) => [r.x, r.y, r.w, r.h]);

function main() {
  const a = process.argv.slice(2); const get = (k) => { const i = a.indexOf(k); return i >= 0 ? a[i + 1] : undefined; };
  const dir = resolve(repo, get('--dir') ?? 'docs/screenshots/headless');
  const ref = get('--base') ?? 'HEAD';
  const out = resolve(repo, get('--out') ?? 'docs/screenshots/diff');
  const strict = a.includes('--strict');
  const ignoreByName = Object.fromEntries(loadManifest().map((e) => [e.name, toRects(e.ignore)]));
  mkdirSync(out, { recursive: true });
  const rows = []; let changed = 0, same = 0, fresh = 0;
  for (const f of readdirSync(dir).filter((f) => f.endsWith('.png')).sort()) {
    const rel = join('docs/screenshots/headless', f).replaceAll('\\', '/');
    const cur = readFileSync(join(dir, f)); const base = fromGit(ref, rel);
    const name = f.replace(/\.(faceplate|deck)\.png$/, '');
    if (!base) { fresh++; rows.push({ f, status: 'new' }); continue; }
    let r; try { r = diffPng(base, cur, { ignore: ignoreByName[name] ?? [] }); } catch (e) { rows.push({ f, status: 'error', error: e.message }); changed++; continue; }
    if (r.changed === 0) { same++; rows.push({ f, status: 'same' }); continue; }
    changed++; const d = f.replace(/\.png$/, '.diff.png'); writeFileSync(join(out, d), r.diffPng); writeFileSync(join(out, 'base.' + f), base);
    rows.push({ f, status: 'changed', changed: r.changed, ratio: r.ratio, diff: d });
  }
  const html = `<!doctype html><meta charset="utf-8"><title>Nixon screenshot diff</title>
<style>body{font:14px system-ui;margin:16px;background:#1a1816;color:#e6e1d8}table{border-collapse:collapse}td{padding:6px;vertical-align:top}img{max-width:420px;border:1px solid #444}.same{opacity:.5}</style>
<h1>${changed} changed / ${same} same / ${fresh} new — base ${ref}</h1><table>` +
    rows.map((r) => r.status === 'changed'
      ? `<tr><td>${r.f}<br>${r.changed} px (${(r.ratio * 100).toFixed(2)}%)</td><td><img src="base.${r.f}"></td><td><img src="../headless/${r.f}"></td><td><img src="${r.diff}"></td></tr>`
      : `<tr class="${r.status}"><td>${r.f}</td><td colspan="3">${r.status}${r.error ? ': ' + r.error : ''}</td></tr>`).join('\n') + '</table>';
  writeFileSync(join(out, 'index.html'), html);
  console.log(`${changed} changed / ${same} same / ${fresh} new → ${join(out, 'index.html')}`);
  process.exit(strict && changed ? 1 : 0);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
