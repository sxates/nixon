'use client';

import React, { useEffect, useRef, useState } from 'react';
import { cn } from '@/lib/utils';
import { createVuIntegrator, vuToArcFraction, VU_MIN_DB } from '@/lib/transport/vu-ballistics';

const ANGLE_MIN = -50;
const ANGLE_MAX = 50;
/** Sub-pixel needle moves aren't worth a React render (~0.2° at this size). */
const RENDER_EPSILON_DB = 0.05;

/**
 * specs/0057 §3.2 — needle VU. `db` is the raw target (from rmsToVu); the needle follows it
 * through the ballistics integrator stepped on requestAnimationFrame, so the 12.5 Hz feed
 * never stutters. Under prefers-reduced-motion the needle snaps (no rAF loop).
 */
export function VuMeter({
  db,
  active,
  label,
  className,
}: {
  db: number;
  active: boolean;
  label: string;
  className?: string;
}) {
  const integ = useRef(createVuIntegrator());
  const target = useRef(db);
  const [needleDb, setNeedleDb] = useState(VU_MIN_DB);
  const rendered = useRef(VU_MIN_DB);
  const [reduced, setReduced] = useState(false);
  // The rAF clock lives OUTSIDE the loop effect. The effect re-arms on every `db` change
  // (12.5 Hz), and a `last` local would be re-seeded at re-arm time — mid-frame — so each
  // tick measured only the sliver since the re-arm instead of since the previous step,
  // silently slowing the ballistics by roughly half. A ref carries the real last-step time
  // across re-arms; the dt clamp below still covers a long park.
  const last = useRef(0);
  target.current = active ? db : VU_MIN_DB;

  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    if (!mq) return;
    setReduced(mq.matches);
    // The preference can be flipped while the app is open — follow it instead of latching
    // whatever was true at mount.
    const onChange = (e: MediaQueryListEvent) => setReduced(e.matches);
    mq.addEventListener?.('change', onChange);
    return () => mq.removeEventListener?.('change', onChange);
  }, []);

  // Reduced motion: snap to the target, no animation loop at all.
  useEffect(() => {
    if (!reduced) return;
    integ.current.reset(target.current);
    rendered.current = target.current;
    setNeedleDb(target.current);
  }, [reduced, db, active]);

  // `active`/`db` are effect deps only so the loop RE-ARMS after it parks; the loop itself
  // always reads the live `target` ref, so a re-run never restarts the ballistics.
  useEffect(() => {
    if (reduced) return;
    let raf = 0;
    if (last.current === 0) last.current = performance.now();
    const tick = (now: number) => {
      // Clamp dt: a backgrounded tab parks rAF, and on return an un-clamped delta would make
      // the integrator sub-step thousands of times in one frame.
      const dt = Math.min(now - last.current, 100);
      last.current = now;
      const next = integ.current.step(target.current, dt);
      // Skip the render when the needle barely moved — one meter on the record page
      // shouldn't cost a React commit every frame.
      if (Math.abs(next - rendered.current) >= RENDER_EPSILON_DB) {
        rendered.current = next;
        setNeedleDb(next);
      }
      // Park the loop once the meter is idle and the needle has settled on its rest target:
      // an inactive VU has nothing left to animate, so burning a frame callback forever is
      // pure waste. The effect re-arms whenever `active` or `db` changes.
      if (!active && Math.abs(next - target.current) < RENDER_EPSILON_DB) {
        raf = 0;
        return;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [reduced, active, db]);

  const angle = ANGLE_MIN + vuToArcFraction(needleDb) * (ANGLE_MAX - ANGLE_MIN);
  return (
    <div
      className={cn('flex flex-col items-center', className)}
      role="img"
      aria-label={`${label} level ${Math.round(needleDb)} VU`}
    >
      {/* 132×57 keeps the original 180×78 drawing exactly — same viewBox, same arc, same
          pivot — scaled to 0.733. Owner feedback 2026-09-21: the meter bridge was making the
          record header tall enough to push the transcript down the page, and two meters is
          the one thing on that header that can give back height without losing information.
          The in-SVG font sizes below are pre-divided by that scale so the engraving still
          renders at its intended pixel size. */}
      <div className="relative h-[57px] w-[132px] overflow-hidden rounded-[2px] bg-well shadow-[inset_0_1px_2px_rgba(0,0,0,0.6),inset_0_-1px_0_hsl(var(--bevel-hi))]">
        <svg viewBox="0 0 180 78" className="block h-full w-full">
          <path d="M30.3 44.9 A78 78 0 0 1 119.2 22.7" className="stroke-engrave" strokeWidth="1" fill="none" />
          <path d="M119.2 22.7 A78 78 0 0 1 149.7 44.9" className="stroke-meter-over" strokeWidth="2.5" fill="none" />
          <g className="stroke-engrave" strokeWidth="1">
            <path d="M30.3 44.9 L34.8 48.7" />
            <path d="M51 27.5 L54 32.7" />
            <path d="M69.8 19.7 L71.4 25.5" />
            <path d="M83.2 17.3 L83.7 23.3" />
            <path d="M100.9 17.8 L100 23.7" />
            <path d="M119.2 22.7 L117 28.3" />
            <path d="M145.1 39.9 L140.9 44.1" />
          </g>
          <g className="fill-engrave font-narrow" fontSize="10.9" textAnchor="middle">
            <text x="22.6" y="40">-20</text>
            <text x="46" y="20">-10</text>
            <text x="67.2" y="12">-7</text>
            <text x="82.3" y="9">-5</text>
            <text x="102.2" y="10">-3</text>
            <text x="123" y="15">0</text>
            <text x="152.2" y="35">+3</text>
          </g>
          <line
            x1="90"
            y1="95"
            x2="90"
            y2="17"
            className="stroke-foreground"
            strokeWidth="1.5"
            transform={`rotate(${angle} 90 95)`}
          />
          <text
            x="90"
            y="72"
            className="fill-engrave font-sans"
            fontSize="12.3"
            fontWeight="600"
            letterSpacing="2"
            textAnchor="middle"
          >
            VU
          </text>
          {/* The channel label lives INSIDE the well, bottom-right, sharing the VU
              engraving's baseline (owner feedback 2026-09-21) — it used to be a separate
              `u-section-label` span below the meter, which cost the header a whole text row
              per channel for something that belongs on the faceplate anyway. `role="img"` +
              `aria-label` on the wrapper still carries it for assistive tech, so this text
              is decorative. */}
          <text
            x="175"
            y="72"
            className="fill-engrave font-sans"
            fontSize="11"
            fontWeight="600"
            letterSpacing="1.4"
            textAnchor="end"
          >
            {label.toUpperCase()}
          </text>
        </svg>
      </div>
    </div>
  );
}
