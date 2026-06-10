// Scale helpers shared by the ThreeJS painter (grid lines) and the React
// overlay (tick labels) — one source of truth so they can't disagree.

import type { SamplePoint } from '../engine/types'

/** "Nice" tick positions covering [lo, hi] — 1/2/5 × 10^k steps. */
export function niceTicks(lo: number, hi: number, target = 6): number[] {
  if (!(hi > lo) || !isFinite(lo) || !isFinite(hi)) return []
  const span = hi - lo
  const rawStep = span / target
  const mag = Math.pow(10, Math.floor(Math.log10(rawStep)))
  const norm = rawStep / mag
  const step = (norm >= 5 ? 5 : norm >= 2 ? 2 : 1) * mag
  const ticks: number[] = []
  for (let t = Math.ceil(lo / step) * step; t <= hi + step * 1e-9; t += step) {
    // snap floating-point drift (0.30000000000000004 → 0.3)
    ticks.push(Math.abs(t) < step * 1e-9 ? 0 : parseFloat(t.toPrecision(12)))
  }
  return ticks
}

/** Robust y-domain from samples: 2%–98% quantiles with padding, so one pole
 * spike doesn't flatten the whole curve. */
export function quantileDomain(points: SamplePoint[]): [number, number] {
  const ys = points
    .map((p) => p[1])
    .filter((y): y is number => y !== null)
    .sort((a, b) => a - b)
  if (ys.length === 0) return [-1, 1]
  let lo = ys[Math.floor(ys.length * 0.02)]
  let hi = ys[Math.min(ys.length - 1, Math.floor(ys.length * 0.98))]
  if (lo === hi) {
    lo -= 1
    hi += 1
  }
  const pad = (hi - lo) * 0.08
  return [lo - pad, hi + pad]
}

/** Compact tick label: trims float noise, switches to exponent notation when
 * the magnitude warrants it. */
export function formatTick(v: number): string {
  if (v === 0) return '0'
  const a = Math.abs(v)
  if (a >= 10000 || a < 0.001) return v.toExponential(1)
  return String(parseFloat(v.toPrecision(6)))
}
