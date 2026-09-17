import { describe, it, expect } from 'vitest';
import { PNG } from 'pngjs';
import { diffPng } from '../../scripts/shots/diff.mjs';

function solid(w, h, rgb) { const p = new PNG({ width: w, height: h }); for (let i = 0; i < w * h; i++) { p.data[i*4] = rgb[0]; p.data[i*4+1] = rgb[1]; p.data[i*4+2] = rgb[2]; p.data[i*4+3] = 255; } return PNG.sync.write(p); }

describe('diffPng', () => {
  it('reports zero changes for identical images', () => {
    const a = solid(10, 10, [200, 200, 200]);
    expect(diffPng(a, a).changed).toBe(0);
  });
  it('counts changed pixels and masks ignore regions', () => {
    const a = solid(10, 10, [200, 200, 200]);
    const b = PNG.sync.read(solid(10, 10, [200, 200, 200]));
    for (let x = 0; x < 5; x++) { const i = (0 * 10 + x) * 4; b.data[i] = 0; b.data[i+1] = 0; b.data[i+2] = 0; }
    const bb = PNG.sync.write(b);
    expect(diffPng(a, bb).changed).toBe(5);
    expect(diffPng(a, bb, { ignore: [[0, 0, 5, 1]] }).changed).toBe(0);
  });
  it('throws on size mismatch', () => {
    expect(() => diffPng(solid(10, 10, [0,0,0]), solid(11, 10, [0,0,0]))).toThrow(/size/);
  });
});
