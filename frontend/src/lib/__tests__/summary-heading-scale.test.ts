import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * Summary headings must never be the largest text on the meeting page: they stay below the
 * meeting title. BlockNote sizes headings from `--level` (h1 3em, h2 2em, h3 1.3em) on top of
 * the summary's 14px base, which put `#`/`##` at 42px/28px, over the 22px title. globals.css
 * caps levels 1-3 under `.summary-doc`; this test ties those caps to the title's real size.
 */
const root = resolve(__dirname, '../..');
const css = readFileSync(resolve(root, 'app/globals.css'), 'utf8');
const header = readFileSync(resolve(root, 'components/MeetingDetails/MeetingIdentityHeader.tsx'), 'utf8');

/** Title size in px from the display-state <h1> in MeetingIdentityHeader. */
function titlePx(): number {
  const h1 = header.match(/<h1[\s\S]*?>/)?.[0] ?? '';
  const m = h1.match(/text-\[(\d+(?:\.\d+)?)px\]/);
  if (!m) throw new Error('could not find the meeting title size in MeetingIdentityHeader');
  return Number(m[1]);
}

/** The `--level` px value the summary sets for a heading level (1, 2 or 3). */
function summaryLevelPx(level: 1 | 2 | 3): number {
  const selector =
    level === 1
      ? /\.summary-doc \[data-content-type="heading"\]\[data-level="1"\][^{]*\{[^}]*--level:\s*(\d+(?:\.\d+)?)px/
      : new RegExp(
          `\\.summary-doc \\[data-content-type="heading"\\]\\[data-level="${level}"\\]\\s*\\{[^}]*--level:\\s*(\\d+(?:\\.\\d+)?)px`,
        );
  const m = css.match(selector);
  if (!m) throw new Error(`no px --level override for summary heading level ${level} in globals.css`);
  return Number(m[1]);
}

describe('summary heading scale', () => {
  it('keeps every capped heading level smaller than the meeting title', () => {
    const title = titlePx();
    const levels = [summaryLevelPx(1), summaryLevelPx(2), summaryLevelPx(3)];
    for (const px of levels) expect(px).toBeLessThan(title);
  });

  it('keeps h1 >= h2 >= h3 so the hierarchy still reads', () => {
    const [h1, h2, h3] = [summaryLevelPx(1), summaryLevelPx(2), summaryLevelPx(3)];
    expect(h1).toBeGreaterThanOrEqual(h2);
    expect(h2).toBeGreaterThanOrEqual(h3);
  });

  it('also caps a heading that carries no data-level attribute (BlockNote default h1)', () => {
    expect(css).toMatch(/\.summary-doc \[data-content-type="heading"\]:not\(\[data-level\]\)/);
  });
});
