import { readFileSync } from 'node:fs'; import { join } from 'node:path'; import { describe, it, expect } from 'vitest';
const css = readFileSync(join(__dirname, '..', 'app', 'globals.css'), 'utf8');
describe('sonner toast tokens', () => {
  it('re-bases every sonner variable on a theme token', () => {
    const start = css.indexOf('html [data-sonner-toaster][data-sonner-theme]');
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, css.indexOf('}', start));
    for (const kind of ['success', 'info', 'warning', 'error']) for (const part of ['bg', 'border', 'text']) expect(block).toMatch(new RegExp(`--${kind}-${part}:\\s*hsl\\(var\\(--`));
    for (const part of ['bg', 'border', 'text', 'bg-hover', 'border-hover']) expect(block).toMatch(new RegExp(`--normal-${part}:\\s*hsl\\(var\\(--`));
  });
});
