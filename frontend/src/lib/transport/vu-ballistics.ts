/**
 * specs/0057 §3.2 — VU meter ballistics.
 *
 * A critically-damped-ish second-order system: the needle position `x` chases the target
 * with velocity `v`; `omega` is chosen so a step reaches 99% in `settleMs`. Slight
 * under-damping (zeta ≈ 0.85) gives the classic ~1–1.5% overshoot. Stepped with the real
 * elapsed dt so it converges identically at the 80 ms event cadence and at 60 fps.
 */
export interface VuIntegrator {
  step(targetDb: number, dtMs: number): number;
  value(): number;
  reset(db?: number): void;
}

export const VU_MIN_DB = -20;
export const VU_MAX_DB = 3;

/** Largest explicit-Euler sub-step, seconds. Keeps the 80 ms feed cadence stable. */
const MAX_SUB_STEP_S = 0.008;

export function createVuIntegrator(opts: { settleMs?: number; zeta?: number } = {}): VuIntegrator {
  const settle = opts.settleMs ?? 300;
  const zeta = opts.zeta ?? 0.85;
  // For a 2nd-order system, 1% settling ≈ 4.6 / (zeta·omega) ⇒ omega = 4.6 / (zeta·T)
  const omega = 4.6 / (zeta * (settle / 1000));
  let x = VU_MIN_DB;
  let v = 0;
  return {
    step(target, dtMs) {
      // Sub-step so large dt (80 ms feed) stays stable: cap each integration step at 8 ms.
      let remaining = Math.max(0, dtMs) / 1000;
      while (remaining > 0) {
        const h = Math.min(remaining, MAX_SUB_STEP_S);
        const a = omega * omega * (target - x) - 2 * zeta * omega * v;
        v += a * h;
        x += v * h;
        remaining -= h;
      }
      x = Math.min(VU_MAX_DB + 0.5, Math.max(VU_MIN_DB - 0.5, x));
      return x;
    },
    value: () => x,
    reset(db = VU_MIN_DB) {
      x = db;
      v = 0;
    },
  };
}

/**
 * Alignment level: 0 VU = -18 dBFS (the EBU R68 digital reference). The mic path is
 * EBU R128-normalised to -23 LUFS before it reaches the meter (audio_processing.rs
 * `normalize_loudness`), so an un-referenced 0 VU = 0 dBFS face would park CH1 MIC in
 * the bottom 15% of the arc during normal speech. With -18 dBFS at 0 VU, normalised
 * speech averages around -5 VU and peaks near 0 — the classic reading.
 */
export const VU_REF_DBFS = -18;

/** 20·log10(rms) relative to VU_REF_DBFS, onto the classic face: -20 … +3 VU. */
export function rmsToVu(rms: number): number {
  if (!(rms > 0)) return VU_MIN_DB;
  const db = 20 * Math.log10(rms) - VU_REF_DBFS;
  return Math.min(VU_MAX_DB, Math.max(VU_MIN_DB, db));
}

/** Arc travel 0..1 with 0 VU at 72% (the red zone is the last 28%). Piecewise-linear. */
export function vuToArcFraction(db: number): number {
  const d = Math.min(VU_MAX_DB, Math.max(VU_MIN_DB, db));
  if (d <= 0) return ((d - VU_MIN_DB) / (0 - VU_MIN_DB)) * 0.72;
  return 0.72 + (d / VU_MAX_DB) * 0.28;
}
