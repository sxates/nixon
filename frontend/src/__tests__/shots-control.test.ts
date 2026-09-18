import { describe, it, expect } from 'vitest';
import { frame, parseReply } from '../../scripts/shots/lib/control.mjs';

describe('control protocol', () => {
  it('frames one JSON object per line', () => {
    expect(frame({ cmd: 'ping' })).toBe('{"cmd":"ping"}\n');
  });
  it('parses ok and error replies', () => {
    expect(parseReply('{"ok":true,"ms":3}\n')).toEqual({ ok: true, ms: 3 });
    expect(() => parseReply('{"ok":false,"error":"boom"}\n')).toThrow(/boom/);
  });
});
