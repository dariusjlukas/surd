// React wrapper around SurfacePlot: owns the orbit and wires drag-to-rotate
// and wheel-to-dolly. No engine resampling here — the grid is fixed at eval
// time and orbiting is purely a camera move (change the domain by editing
// the cell).

import { useEffect, useMemo, useRef, useState } from 'react'
import type { Plot3dData } from '../engine/types'
import { useSettings } from '../state/settings'
import { openContextMenu } from '../state/contextMenu'
import { MathInline } from '../components/MathOutput'
import { formatTick, niceTicks } from './scales'
import {
  clampOrbit,
  DEFAULT_ORBIT,
  projectToPx,
  SurfacePlot,
  zRange,
  Z_SCALE,
  type Orbit,
} from './SurfacePlot'

interface AxisLabel {
  key: string
  text: string
  /** Scene-space anchor, slightly outside the unit box. */
  p: [number, number, number]
  /** Axis names render larger than tick values. */
  isName?: boolean
}

/** Tick + name labels anchored to the box edges facing the camera, so they
 * stay readable (and out of the surface's way) as the user orbits. */
function axisLabels(plot: Plot3dData, zlo: number, zhi: number, azimuth: number): AxisLabel[] {
  // Which ±x / ±z box sides the camera is on (scene coords).
  const xSide = Math.sin(azimuth) >= 0 ? 1 : -1
  const zSide = Math.cos(azimuth) >= 0 ? 1 : -1
  // data → scene (see SurfacePlot.setData: data (x, y, z) → THREE (x, z, -y))
  const sx = (t: number) => -1 + (2 * (t - plot.a)) / (plot.b - plot.a)
  const sy = (t: number) => 1 - (2 * (t - plot.c)) / (plot.d - plot.c)
  const sz = (t: number) => -Z_SCALE + (2 * Z_SCALE * (t - zlo)) / (zhi - zlo)

  const out: AxisLabel[] = []
  // The two bottom edges meet at the corner nearest the camera — the lowest
  // point in the projection. Skip ticks within reach of it: they clip at the
  // frame edge and the two tick rows would collide there anyway.
  for (const t of niceTicks(plot.a, plot.b, 5)) {
    if (sx(t) * xSide > 0.9) continue
    out.push({ key: `x${t}`, text: formatTick(t), p: [sx(t), -Z_SCALE - 0.05, zSide * 1.14] })
  }
  for (const t of niceTicks(plot.c, plot.d, 5)) {
    if (sy(t) * zSide > 0.9) continue
    out.push({ key: `y${t}`, text: formatTick(t), p: [xSide * 1.14, -Z_SCALE - 0.05, sy(t)] })
  }
  for (const t of niceTicks(zlo, zhi, 4)) {
    out.push({ key: `z${t}`, text: formatTick(t), p: [-xSide * 1.08, sz(t), zSide * 1.08] })
  }
  // Names push outward along the edge normal, not downward — below-the-box
  // anchors fall off the bottom of the frame at the default orbit.
  out.push({ key: 'xname', text: plot.xvar, isName: true, p: [0, -Z_SCALE - 0.08, zSide * 1.38] })
  out.push({ key: 'yname', text: plot.yvar, isName: true, p: [xSide * 1.38, -Z_SCALE - 0.08, 0] })
  out.push({
    key: 'zname',
    text: 'z',
    isName: true,
    p: [-xSide * 1.14, Z_SCALE + 0.16, zSide * 1.14],
  })
  return out
}

export function Surface3DView({ plot }: { plot: Plot3dData }) {
  const themeKey = useSettings((s) => `${s.resolvedMode}/${s.accent}`)

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const painterRef = useRef<SurfacePlot | null>(null)
  const dragRef = useRef<{ pointerId: number; lastX: number; lastY: number } | null>(null)

  const [orbit, setOrbit] = useState<Orbit>(DEFAULT_ORBIT)
  const [size, setSize] = useState({ w: 640, h: 320 })
  const [zlo, zhi] = useMemo(() => zRange(plot.heights), [plot])
  /** Measurement cursor: the grid node under the pointer. `z` is the true
   * sampled height (the mesh clamps spikes to the box; the readout doesn't);
   * `scene` is where the marker sits, on the (clamped) mesh. */
  const [probe, setProbe] = useState<{
    x: number
    y: number
    z: number
    scene: [number, number, number]
  } | null>(null)

  const labels = useMemo(
    () => axisLabels(plot, zlo, zhi, orbit.azimuth),
    [plot, zlo, zhi, orbit.azimuth],
  )
  const positions = useMemo(
    () => projectToPx(labels.map((l) => l.p), orbit, size.w, size.h),
    [labels, orbit, size],
  )
  const probePos = probe ? projectToPx([probe.scene], orbit, size.w, size.h)[0] : null

  // -- painter lifecycle ----------------------------------------------------
  useEffect(() => {
    const painter = new SurfacePlot(canvasRef.current!)
    painterRef.current = painter
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect
      if (width > 0 && height > 0) {
        painter.resize(width, height)
        setSize({ w: width, h: height })
      }
    })
    ro.observe(frameRef.current!)
    return () => {
      ro.disconnect()
      painter.dispose()
      painterRef.current = null
    }
  }, [])

  useEffect(() => {
    painterRef.current?.setData(plot.heights, plot.nx, plot.ny, zlo, zhi)
  }, [plot, zlo, zhi, themeKey])

  useEffect(() => {
    painterRef.current?.setOrbit(orbit)
  }, [orbit])

  // -- drag to rotate, wheel to dolly ---------------------------------------
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return
    dragRef.current = { pointerId: e.pointerId, lastX: e.clientX, lastY: e.clientY }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  /** Raycast the pointer into the mesh and snap to the nearest grid node —
   * node values are the honest sampled data (no interpolation invented). */
  const updateProbe = (e: React.PointerEvent) => {
    const rect = frameRef.current!.getBoundingClientRect()
    const hit = painterRef.current?.pick(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -(((e.clientY - rect.top) / rect.height) * 2 - 1),
    )
    if (!hit) {
      setProbe(null)
      return
    }
    // invert the scene mapping (see SurfacePlot.setData): scene x = xn,
    // scene z = -yn, scene y = clamped height band
    const i = Math.round(((hit.x + 1) / 2) * (plot.nx - 1))
    const j = Math.round(((1 - hit.z) / 2) * (plot.ny - 1))
    const h = plot.heights[j * plot.nx + i]
    if (h === null || h === undefined) {
      setProbe(null)
      return
    }
    const t = Math.min(1, Math.max(0, (h - zlo) / (zhi - zlo)))
    setProbe({
      x: plot.a + (i / (plot.nx - 1)) * (plot.b - plot.a),
      y: plot.c + (j / (plot.ny - 1)) * (plot.d - plot.c),
      z: h,
      scene: [
        -1 + (2 * i) / (plot.nx - 1),
        (2 * t - 1) * Z_SCALE,
        1 - (2 * j) / (plot.ny - 1),
      ],
    })
  }

  const onPointerMove = (e: React.PointerEvent) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== e.pointerId) {
      updateProbe(e)
      return
    }
    setProbe(null)
    const dx = e.clientX - drag.lastX
    const dy = e.clientY - drag.lastY
    drag.lastX = e.clientX
    drag.lastY = e.clientY
    setOrbit((o) =>
      clampOrbit({
        azimuth: o.azimuth - dx * 0.008,
        elevation: o.elevation + dy * 0.008,
        radius: o.radius,
      }),
    )
  }

  const onPointerUp = (e: React.PointerEvent) => {
    if (dragRef.current?.pointerId === e.pointerId) dragRef.current = null
  }

  useEffect(() => {
    const el = frameRef.current!
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const factor = Math.exp(e.deltaY * 0.0015)
      setOrbit((o) => clampOrbit({ ...o, radius: o.radius * factor }))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [])

  const reset = () => setOrbit(DEFAULT_ORBIT)

  const savePng = () => {
    const painter = painterRef.current
    if (!painter) return
    const a = document.createElement('a')
    a.href = painter.snapshot()
    a.download = `${plot.text.slice(0, 40)}.png`
    a.click()
  }

  return (
    <div className="max-w-2xl">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
        <MathInline latex={plot.latex} fallback={plot.text} />
        <span className="text-xs">
          {plot.xvar} ∈ [{formatTick(plot.a)}, {formatTick(plot.b)}]
        </span>
        <span className="text-xs">
          {plot.yvar} ∈ [{formatTick(plot.c)}, {formatTick(plot.d)}]
        </span>
        <span className="text-xs" title="2%–98% quantile range; spikes clamp to the box">
          z ∈ [{formatTick(zlo)}, {formatTick(zhi)}]
        </span>
        <button
          onClick={savePng}
          title="download the plot as a PNG"
          className="ml-auto rounded-md border border-edge-strong px-2 text-xs text-muted hover:text-ink"
        >
          png
        </button>
        <button
          onClick={reset}
          className="rounded-md border border-edge-strong px-2 text-xs text-muted hover:text-ink"
        >
          reset
        </button>
      </div>
      <div
        ref={frameRef}
        title="drag to rotate · wheel zooms"
        className="relative h-80 cursor-grab touch-none select-none overflow-hidden rounded-lg border border-edge bg-surface active:cursor-grabbing"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onPointerLeave={() => setProbe(null)}
        onDoubleClick={reset}
        onContextMenu={(e) =>
          openContextMenu(e, [
            { label: 'Save as PNG', onSelect: savePng },
            { label: 'Reset view', onSelect: reset },
            'divider',
            {
              label: 'Copy expression',
              onSelect: () => void navigator.clipboard.writeText(plot.text),
            },
          ])
        }
      >
        <canvas ref={canvasRef} className="block h-full w-full" />
        {labels.map((l, i) =>
          positions[i].visible ? (
            <span
              key={l.key}
              className={`pointer-events-none absolute -translate-x-1/2 -translate-y-1/2 font-mono ${
                l.isName ? 'text-[11px] text-muted' : 'text-[10px] text-faint'
              }`}
              style={{ left: positions[i].x, top: positions[i].y }}
            >
              {l.text}
            </span>
          ) : null,
        )}
        {probe && probePos?.visible && (
          <>
            <div
              className="pointer-events-none absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-accent bg-app"
              style={{ left: probePos.x, top: probePos.y }}
            />
            <div
              className="pointer-events-none absolute z-10 whitespace-nowrap rounded-md border border-edge bg-raised px-1.5 py-0.5 font-mono text-[11px] text-ink"
              style={{
                left: probePos.x + (probePos.x > size.w - 180 ? -10 : 10),
                top: probePos.y - 26,
                transform: probePos.x > size.w - 180 ? 'translateX(-100%)' : undefined,
              }}
            >
              ({formatTick(probe.x)}, {formatTick(probe.y)}, {formatTick(probe.z)})
            </div>
          </>
        )}
      </div>
    </div>
  )
}
