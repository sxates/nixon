// render.mjs — draws the menu bar (tray) icons: one tape reel, idle / recording / paused.
//
//   node scripts/tray-icons/render.mjs      (from frontend/, needs Google Chrome)
//
// Writes src-tauri/icons/tray/*.png at 44×44 (22pt @2x), all black on transparent: macOS
// shows them as template images and tints them to match the menu bar and wallpaper. The reel
// is the app icon's reel (the take-up hub in components/Transport/Reels.tsx).
//   idle.png          the reel at rest
//   rec-00..15.png    the reel turned 7.5° per frame (the teeth repeat every 120°), with a gap
//                     cut in the flange where the red REC light sits. A template can't hold
//                     red, so the light itself is drawn by the app over the icon
//                     (src/tray_reel.rs); keep LIGHT in step with LIGHT_* there.
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import puppeteer from 'puppeteer-core';

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, '../../src-tauri/icons/tray');
mkdirSync(out, { recursive: true });

const INK = '#000';
const FRAMES = 16;
const STEP = 120 / FRAMES;
// Reel centre and the REC light, in a 22×22 box.
const C = { x: 10, y: 12 };
const LIGHT = { x: 18.4, y: 3.8, r: 2.5 };

// The take-up hub in Reels.tsx, which is also the reel in the app icon (icons/nixon-icon.svg):
// a heavy flange, a small hub ring, and three thick teeth from the hub most of the way to
// the flange. Proportions follow the icon (flange r78 / hub r30 / teeth to r66), scaled to
// r8.6 here, with the strokes eased a little so they stay crisp at menu bar size.
function reel(angle) {
  const teeth = [0, 120, 240]
    .map((a) => `<path d="M${C.x} ${C.y - 3.3} L${C.x} ${C.y - 7.1}" transform="rotate(${a + angle} ${C.x} ${C.y})"/>`)
    .join('');
  return `
    <circle cx="${C.x}" cy="${C.y}" r="8.4" fill="none" stroke="${INK}" stroke-width="1.9"/>
    <circle cx="${C.x}" cy="${C.y}" r="3.3" fill="none" stroke="${INK}" stroke-width="1.3"/>
    <g stroke="${INK}" stroke-width="1.9" stroke-linecap="butt">${teeth}</g>`;
}

// The light sits over the flange's corner, so the reel is knocked out around it.
function svg(angle, gap) {
  const mask = gap
    ? `<defs><mask id="m"><rect width="22" height="22" fill="#fff"/><circle cx="${LIGHT.x}" cy="${LIGHT.y}" r="${LIGHT.r + 1.3}" fill="#000"/></mask></defs>`
    : '';
  return `<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 22 22">
    ${mask}<g ${gap ? 'mask="url(#m)"' : ''}>${reel(angle)}</g></svg>`;
}

const icons = [
  ['idle.png', svg(0, false)],
  ...Array.from({ length: FRAMES }, (_, i) => [`rec-${String(i).padStart(2, '0')}.png`, svg(i * STEP, true)]),
];

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
});
const page = await browser.newPage();
await page.setViewport({ width: 44, height: 44, deviceScaleFactor: 1 });
for (const [name, markup] of icons) {
  await page.setContent(`<html><body style="margin:0;background:transparent">${markup}</body></html>`);
  const el = await page.$('svg');
  writeFileSync(join(out, name), await el.screenshot({ omitBackground: true }));
  console.log('wrote', name);
}
await browser.close();
