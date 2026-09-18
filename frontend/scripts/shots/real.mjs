#!/usr/bin/env node
// real.mjs — drive the running Dev Nixon window and capture it with screencapture (specs/0060).
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadManifest, expand } from './lib/manifest.mjs';
import { connect, readPort } from './lib/control.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..', '..');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function args() {
  const a = process.argv.slice(2);
  const get = (k) => {
    const i = a.indexOf(k);
    return i >= 0 ? a[i + 1] : undefined;
  };
  const audioDefault = ['meetily-recordings', 'nixon-recordings']
    .map((d) => join(homedir(), 'Movies', d, 'nixon-demo-01', 'audio.mp4'))
    .find(existsSync);
  return {
    out: get('--out') ?? join(repo, 'docs', 'screenshots', 'real'),
    only: get('--only')?.split(','),
    theme: get('--theme'),
    audio: get('--audio') ?? audioDefault,
    port: get('--port') ? Number(get('--port')) : readPort(),
  };
}

async function main() {
  const opt = args();
  const c = connect(opt.port);
  await c.ready;
  await c.send({ cmd: 'ping' });
  mkdirSync(opt.out, { recursive: true });
  let shots = expand(loadManifest());
  if (opt.only) shots = shots.filter((s) => opt.only.includes(s.name));
  if (opt.theme) shots = shots.filter((s) => s.theme === opt.theme);
  let currentTheme = null;
  let failures = 0;
  // Tracked so SIGINT/SIGTERM can clean up a take that's actually in progress.
  let currentPlayer = null;
  let recordingActive = false;

  const onSignal = (signal) => () => {
    console.error(`\n${signal} — cleaning up`);
    currentPlayer?.kill();
    (recordingActive ? c.send({ cmd: 'stop_recording' }).catch(() => {}) : Promise.resolve()).finally(() =>
      process.exit(130),
    );
  };
  process.on('SIGINT', onSignal('SIGINT'));
  process.on('SIGTERM', onSignal('SIGTERM'));

  await c.send({ cmd: 'hide_dev_badge', value: true });
  try {
    for (const shot of shots) {
      try {
        const [w, h] = shot.entry.viewport;
        await c.send({ cmd: 'resize', w, h });
        if (currentTheme !== shot.theme) {
          await c.send({ cmd: 'theme', value: shot.theme === 'deck' ? 'dark' : 'light' });
          await c.send({ cmd: 'ready' });
          currentTheme = shot.theme;
        }
        if (shot.entry.onboardingStep) await c.send({ cmd: 'onboarding_step', value: shot.entry.onboardingStep });
        else await c.send({ cmd: 'navigate', route: shot.entry.route });
        await c.send({ cmd: 'ready' });
        await c.send({ cmd: 'hide_dev_badge', value: true });
        if (shot.entry.state === 'recording') {
          // Ruling (Task 5 review): make sure no real meeting is selected before
          // start_recording — a control-driven stop's `recording-stopped` listener
          // would otherwise overwrite that meeting's cached folder path.
          await c.send({ cmd: 'navigate', route: '/' });
          await c.send({ cmd: 'ready' });
          await c.send({ cmd: 'start_recording', title: 'Screenshot take' });
          recordingActive = true;
          // `navigate '/'` above was a full reload (wipes hide_dev_badge's DOM flag) —
          // return to the shot's real route (e.g. /record) and re-hide the badge before
          // capturing, or the PNG would show Home with the DEV badge visible.
          await c.send({ cmd: 'navigate', route: shot.entry.route });
          await c.send({ cmd: 'ready' });
          await c.send({ cmd: 'hide_dev_badge', value: true });
          if (opt.audio) currentPlayer = spawn('afplay', [opt.audio], { stdio: 'ignore' });
        }
        await sleep(shot.entry.wait);
        try {
          const { window_number } = await c.send({ cmd: 'window' });
          execFileSync('screencapture', ['-l', String(window_number), '-o', '-x', join(opt.out, shot.file)]);
          console.log('ok  ', shot.file, `(route=${shot.entry.route})`);
        } finally {
          if (shot.entry.state === 'recording') {
            currentPlayer?.kill();
            currentPlayer = null;
            await c.send({ cmd: 'stop_recording' });
            recordingActive = false;
          }
        }
      } catch (e) {
        failures++;
        console.error('ERR ', shot.file, e.message);
      }
    }
  } finally {
    // Structural: this runs whether the loop finished cleanly or something above threw
    // past the per-shot try/catch, so onboarding/the badge are never left mid-flight.
    if (shots.some((s) => s.entry.onboardingStep)) {
      await c.send({ cmd: 'onboarding_complete' }).catch(() => {});
      await c.send({ cmd: 'ready' }).catch(() => {});
    }
    await c.send({ cmd: 'hide_dev_badge', value: false }).catch(() => {});
    c.close();
  }
  process.exit(failures ? 1 : 0);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
