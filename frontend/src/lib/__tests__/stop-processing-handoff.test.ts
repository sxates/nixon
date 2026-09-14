import { describe, it, expect } from 'vitest';
import { stopFollowUp, type HandoffOutcome } from '@/lib/stop-processing-handoff';

// spec 0051 WS2 — the stop path used to dispatch a fire-and-forget window event for a
// 'process-now' meeting while suppressing BOTH fallbacks (auto-diarize and
// auto-summary). One missed dispatch meant nothing ran and nothing said so. This
// module is the decision that replaces that assumption.

describe('stopFollowUp', () => {
  it('leaves a plain live meeting on the normal auto-diarize path', () => {
    expect(stopFollowUp('none', null)).toEqual({
      autoDiarize: true,
      deferMarker: false,
      toast: null,
    });
  });

  it('marks a still-deferred meeting and runs nothing at stop', () => {
    expect(stopFollowUp('mark-defer', null)).toEqual({
      autoDiarize: false,
      deferMarker: true,
      toast: null,
    });
  });

  it('stays quiet when the process-now handoff was accepted', () => {
    const handoff: HandoffOutcome = { accepted: true };
    expect(stopFollowUp('process-now', handoff)).toEqual({
      autoDiarize: false,
      deferMarker: true,
      toast: null,
    });
  });

  it('falls back to auto-diarize and warns when the handoff is refused', () => {
    const handoff: HandoffOutcome = { accepted: false, reason: 'no-folder-path' };
    const result = stopFollowUp('process-now', handoff);
    expect(result.autoDiarize).toBe(true);
    expect(result.deferMarker).toBe(true);
    expect(result.toast).not.toBeNull();
  });

  it('falls back the same way when the handoff threw', () => {
    const handoff: HandoffOutcome = { accepted: false, reason: 'threw' };
    expect(stopFollowUp('process-now', handoff)).toEqual(
      stopFollowUp('process-now', { accepted: false, reason: 'no-folder-path' }),
    );
  });

  it('treats a missing handoff result for process-now as a failure, not a success', () => {
    const result = stopFollowUp('process-now', null);
    expect(result.autoDiarize).toBe(true);
    expect(result.toast).not.toBeNull();
  });

  it('always keeps the defer marker for a process-now meeting so a later pass can retry', () => {
    for (const handoff of [
      { accepted: true } as HandoffOutcome,
      { accepted: false, reason: 'threw' } as HandoffOutcome,
      null,
    ]) {
      expect(stopFollowUp('process-now', handoff).deferMarker).toBe(true);
    }
  });
});
