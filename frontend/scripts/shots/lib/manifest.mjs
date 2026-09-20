// manifest.mjs — load, validate and expand scripts/shots/routes.json (specs/0060).
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
export const DEFAULTS = { viewport: [1280, 860], themes: ['faceplate', 'deck'], wait: 1500 };
const THEMES = new Set(['faceplate', 'deck']);
const UPDATE_STATES = new Set(['ready', 'downloading']);

export function loadManifest(path = join(here, '..', 'routes.json')) {
  const raw = JSON.parse(readFileSync(path, 'utf8'));
  if (!Array.isArray(raw)) throw new Error('routes.json must be an array');
  const seen = new Set();
  return raw.map((e, i) => {
    if (!e.name || !e.route) throw new Error(`entry #${i} needs name and route`);
    if (seen.has(e.name)) throw new Error(`duplicate name ${e.name}`);
    seen.add(e.name);
    if (!e.route.startsWith('/')) throw new Error(`${e.name}: route must start with /`);
    const themes = e.themes ?? DEFAULTS.themes;
    for (const t of themes) if (!THEMES.has(t)) throw new Error(`${e.name}: unknown theme ${t}`);
    if (e.onboardingStep !== undefined && !(e.onboardingStep >= 1 && e.onboardingStep <= 5)) throw new Error(`${e.name}: onboardingStep 1..5`);
    if (e.update !== undefined && !UPDATE_STATES.has(e.update)) throw new Error(`${e.name}: unknown update state ${e.update}`);
    return { ...e, viewport: e.viewport ?? DEFAULTS.viewport, themes, wait: e.wait ?? DEFAULTS.wait, ignore: e.ignore ?? [] };
  });
}

export const outputName = (name, theme) => `${name}.${theme}.png`;

export function expand(entries, { headless = false } = {}) {
  const shots = [];
  for (const entry of entries) {
    if (headless && entry.real_only) continue;
    for (const theme of entry.themes) {
      shots.push({
        name: entry.name, theme, entry, file: outputName(entry.name, theme),
        url(base) {
          const u = new URL(entry.route, base);
          u.searchParams.set('theme', theme);
          if (entry.onboardingStep) u.searchParams.set('onboardingStep', String(entry.onboardingStep));
          if (entry.sidebar) u.searchParams.set('sidebar', entry.sidebar);
          // specs/0069 review, fix round 2 — a per-route flag so exactly one route stages
          // an update, rather than every route growing a restart glyph (build-mock.mjs
          // reads this to answer api_get_update_status).
          if (entry.update) u.searchParams.set('update', entry.update);
          return u.toString();
        },
      });
    }
  }
  return shots;
}
