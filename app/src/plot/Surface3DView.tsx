// React wrapper around SurfacePlot: owns the orbit AND the data domain.
// Drag rotates (a camera move); shift+drag pans the domain and wheel zooms
// it about the cursor — both re-sample the surface from the engine, exactly
// like the 2D plot's pan/zoom (alt+wheel still dollies the camera). Resample
// requests are throttled to one in flight with the latest window queued, so
// continuous pans track at engine speed instead of debounce cadence.

import { useEffect, useMemo, useRef, useState } from 'react'
import type { Plot3dData } from '../engine/types'
import { useSettings, type SurfaceRender } from '../state/settings'
import { useNotebook } from '../state/store'
import { openContextMenu } from '../state/contextMenu'
import { MathInline, MathText } from '../components/MathOutput'
import { nameToLatex } from '../engine/nameLatex'
import { exportPlotPng } from './exportPng'
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
import { registerPlotSnapshot } from './snapshots'
import { saveDataUrl } from '../platform/desktop'

/** Header toggle for the surface draw style (see SurfaceRender). */
const RENDER_MODES: { id: SurfaceRender; label: string; title: string }[] = [
  { id: 'solid', label: 'solid', title: 'opaque shaded surface' },
  { id: 'glass', label: 'glass', title: 'semi-transparent surface' },
  { id: 'wire', label: 'wire', title: 'wireframe mesh' },
]

/** The data domain currently on screen: [a, b]×[c, d]. */
interface Win {
  a: number
  b: number
  c: number
  d: number
}

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
function axisLabels(
  plot: Plot3dData,
  win: Win,
  zlo: number,
  zhi: number,
  azimuth: number,
): AxisLabel[] {
  // Which ±x / ±z box sides the camera is on (scene coords).
  const xSide = Math.sin(azimuth) >= 0 ? 1 : -1
  const zSide = Math.cos(azimuth) >= 0 ? 1 : -1
  // data → scene (see SurfacePlot.setData: data (x, y, z) → THREE (x, z, -y))
  const sx = (t: number) => -1 + (2 * (t - win.a)) / (win.b - win.a)
  const sy = (t: number) => 1 - (2 * (t - win.c)) / (win.d - win.c)
  const sz = (t: number) => -Z_SCALE + (2 * Z_SCALE * (t - zlo)) / (zhi - zlo)

  const out: AxisLabel[] = []
  // The two bottom edges meet at the corner nearest the camera — the lowest
  // point in the projection. Skip ticks within reach of it: they clip at the
  // frame edge and the two tick rows would collide there anyway.
  for (const t of niceTicks(win.a, win.b, 5)) {
    if (sx(t) * xSide > 0.9) continue
    out.push({
      key: `x${t}`,
      text: formatTick(t),
      p: [sx(t), -Z_SCALE - 0.05, zSide * 1.14],
    })
  }
  for (const t of niceTicks(win.c, win.d, 5)) {
    if (sy(t) * zSide > 0.9) continue
    out.push({
      key: `y${t}`,
      text: formatTick(t),
      p: [xSide * 1.14, -Z_SCALE - 0.05, sy(t)],
    })
  }
  for (const t of niceTicks(zlo, zhi, 4)) {
    out.push({
      key: `z${t}`,
      text: formatTick(t),
      p: [-xSide * 1.08, sz(t), zSide * 1.08],
    })
  }
  // Names push outward along the edge normal, not downward — below-the-box
  // anchors fall off the bottom of the frame at the default orbit.
  out.push({
    key: 'xname',
    text: plot.xvar,
    isName: true,
    p: [0, -Z_SCALE - 0.08, zSide * 1.38],
  })
  out.push({
    key: 'yname',
    text: plot.yvar,
    isName: true,
    p: [xSide * 1.38, -Z_SCALE - 0.08, 0],
  })
  out.push({
    key: 'zname',
    text: 'z',
    isName: true,
    p: [-xSide * 1.14, Z_SCALE + 0.16, zSide * 1.14],
  })
  return out
}

/** The surface currently drawn: heights + the grid they index by + the
 * window they were sampled over, updated atomically (a resample may come
 * back at a different adaptive resolution, and heights from one grid must
 * never be indexed by another's nx — nor placed by another's window). The
 * sample window lags the view window while a resample is in flight; the
 * painter's setView bridges the gap. */
interface Sampled {
  heights: (number | null)[]
  nx: number
  ny: number
  undersampled: boolean
  win: Win
}

const sampledOf = (p: Plot3dData): Sampled => ({
  heights: p.heights,
  nx: p.nx,
  ny: p.ny,
  undersampled: p.undersampled ?? false,
  win: { a: p.a, b: p.b, c: p.c, d: p.d },
})

type Drag =
  | { mode: 'rotate'; pointerId: number; lastX: number; lastY: number }
  /** Domain pan holds the grabbed data point — each move shifts the window
   * so that point stays under the cursor (no incremental drift). */
  | { mode: 'pan'; pointerId: number; grabX: number; grabY: number }

export function Surface3DView({
  plot,
  cellId,
}: {
  plot: Plot3dData
  /** Owning cell id, when shown in a notebook cell: registers this live view
   * for PDF export (see plot/snapshots). Absent for previews/tests. */
  cellId?: string
}) {
  const resample3d = useNotebook((s) => s.resample3d)
  const themeKey = useSettings((s) => `${s.resolvedMode}/${s.accent}`)
  const surfaceRender = useSettings((s) => s.surfaceRender)
  const setSurfaceRender = useSettings((s) => s.setSurfaceRender)

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const painterRef = useRef<SurfacePlot | null>(null)
  const dragRef = useRef<Drag | null>(null)
  const inFlightRef = useRef(false)
  const latestWinRef = useRef<Win | null>(null)

  const [orbit, setOrbit] = useState<Orbit>(DEFAULT_ORBIT)
  const [size, setSize] = useState({ w: 640, h: 320 })
  const [win, setWin] = useState<Win>({
    a: plot.a,
    b: plot.b,
    c: plot.c,
    d: plot.d,
  })
  const [surf, setSurf] = useState<Sampled>(() => sampledOf(plot))
  /** 3D scatter markers in data coords (static — never resampled). */
  const scatterData = useMemo(() => plot.scatter ?? [], [plot])
  const scatterZs = useMemo(() => scatterData.map((p) => p[2]), [scatterData])
  const [zlo, zhi] = useMemo(
    () => zRange(surf.heights, scatterZs),
    [surf.heights, scatterZs],
  )
  /** No surface to sample (a points-only plot): skip resampling, box the view
   * from the data, and probe only the markers. */
  const pointsOnly = plot.nx < 2
  /** Scatter data → scene coords, mapped over the SAMPLE window so the
   * painter's setView slides the markers in lockstep with the (possibly stale)
   * mesh on pan/zoom — identity once a resample lands. */
  const sceneScatter = useMemo(
    () =>
      scatterData.map(([px, py, pz]) => {
        const t = Math.min(1, Math.max(0, (pz - zlo) / (zhi - zlo)))
        const sw = surf.win
        return [
          -1 + (2 * (px - sw.a)) / (sw.b - sw.a),
          (2 * t - 1) * Z_SCALE,
          1 - (2 * (py - sw.c)) / (sw.d - sw.c),
        ] as [number, number, number]
      }),
    [scatterData, surf.win, zlo, zhi],
  )
  /** Measurement cursor: the grid node under the pointer. `z` is the true
   * sampled height (the mesh clamps spikes to the box; the readout doesn't);
   * `scene` is where the marker sits, on the (clamped) mesh. */
  const [probe, setProbe] = useState<{
    x: number
    y: number
    z: number
    scene: [number, number, number]
  } | null>(null)

  // A re-evaluated cell hands the (long-lived) component a new plot — reset
  // during render (React's "adjusting state when a prop changes" pattern).
  const [prevPlot, setPrevPlot] = useState(plot)
  if (plot !== prevPlot) {
    setPrevPlot(plot)
    setWin({ a: plot.a, b: plot.b, c: plot.c, d: plot.d })
    setSurf(sampledOf(plot))
  }

  const labels = useMemo(
    () => axisLabels(plot, win, zlo, zhi, orbit.azimuth),
    [plot, win, zlo, zhi, orbit.azimuth],
  )
  const positions = useMemo(
    () =>
      projectToPx(
        labels.map((l) => l.p),
        orbit,
        size.w,
        size.h,
      ),
    [labels, orbit, size],
  )
  const probePos = probe
    ? projectToPx([probe.scene], orbit, size.w, size.h)[0]
    : null

  // scene ([-1,1] box) ⇄ data (current window)
  const dataX = (sceneX: number) => win.a + ((sceneX + 1) / 2) * (win.b - win.a)
  const dataY = (sceneZ: number) => win.c + ((1 - sceneZ) / 2) * (win.d - win.c)

  /** One resample in flight; the newest window fires as soon as the previous
   * answer lands. Stale heights stay visible meanwhile, like the 2D plot. */
  const requestResample = (target: Win) => {
    if (pointsOnly) return // no surface expression to sample
    latestWinRef.current = target
    if (inFlightRef.current) return
    inFlightRef.current = true
    const fire = () => {
      const w = latestWinRef.current
      if (!w) {
        inFlightRef.current = false
        return
      }
      resample3d(plot.text, plot.xvar, plot.yvar, w.a, w.b, w.c, w.d)
        .then((r) => {
          // apply only the latest request — a response that was superseded
          // (or belongs to a re-evaluated cell's old plot) is dropped
          if (latestWinRef.current === w)
            setSurf({
              heights: r.heights,
              nx: r.n,
              ny: r.n,
              undersampled: r.undersampled,
              win: w,
            })
        })
        .catch(() => {
          // engine busy or restarted — stale heights stay visible, the next
          // interaction tries again
        })
        .finally(() => {
          if (latestWinRef.current !== w) {
            fire()
          } else {
            inFlightRef.current = false
          }
        })
    }
    fire()
  }

  const ndcOf = (e: { clientX: number; clientY: number }): [number, number] => {
    const rect = frameRef.current!.getBoundingClientRect()
    return [
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -(((e.clientY - rect.top) / rect.height) * 2 - 1),
    ]
  }

  // -- drag: rotate (plain) or pan the domain (shift) ------------------------
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return
    if (e.shiftKey) {
      const hit = painterRef.current?.pickFloor(...ndcOf(e))
      if (!hit) return
      dragRef.current = {
        mode: 'pan',
        pointerId: e.pointerId,
        grabX: dataX(hit.x),
        grabY: dataY(hit.z),
      }
    } else {
      dragRef.current = {
        mode: 'rotate',
        pointerId: e.pointerId,
        lastX: e.clientX,
        lastY: e.clientY,
      }
    }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  /** Raycast the pointer into the mesh and snap to the nearest grid node —
   * node values are the honest sampled data (no interpolation invented). */
  const updateProbe = (e: React.PointerEvent) => {
    const ndc = ndcOf(e)
    // Scatter markers take precedence: snap to the nearest one under the ray
    // and read off its exact (x, y, z).
    const pi = painterRef.current?.pickPoint(...ndc)
    if (pi != null && scatterData[pi]) {
      const [px, py, pz] = scatterData[pi]
      const t = Math.min(1, Math.max(0, (pz - zlo) / (zhi - zlo)))
      setProbe({
        x: px,
        y: py,
        z: pz,
        scene: [
          -1 + (2 * (px - win.a)) / (win.b - win.a),
          (2 * t - 1) * Z_SCALE,
          1 - (2 * (py - win.c)) / (win.d - win.c),
        ],
      })
      return
    }
    const hit = painterRef.current?.pick(...ndc)
    if (!hit) {
      setProbe(null)
      return
    }
    // the hit is on the (possibly stale, view-transformed) mesh: scene →
    // data through the VIEW window, snap to the nearest node of the SAMPLE
    // grid, then map that node back through the view window for the marker
    const sw = surf.win
    const i = Math.round(
      ((dataX(hit.x) - sw.a) / (sw.b - sw.a)) * (surf.nx - 1),
    )
    const j = Math.round(
      ((dataY(hit.z) - sw.c) / (sw.d - sw.c)) * (surf.ny - 1),
    )
    const h =
      i >= 0 && i < surf.nx && j >= 0 && j < surf.ny
        ? surf.heights[j * surf.nx + i]
        : null
    if (h === null || h === undefined) {
      setProbe(null)
      return
    }
    const t = Math.min(1, Math.max(0, (h - zlo) / (zhi - zlo)))
    const px = sw.a + (i / (surf.nx - 1)) * (sw.b - sw.a)
    const py = sw.c + (j / (surf.ny - 1)) * (sw.d - sw.c)
    setProbe({
      x: px,
      y: py,
      z: h,
      scene: [
        -1 + (2 * (px - win.a)) / (win.b - win.a),
        (2 * t - 1) * Z_SCALE,
        1 - (2 * (py - win.c)) / (win.d - win.c),
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
    if (drag.mode === 'rotate') {
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
      return
    }
    // pan: keep the grabbed data point under the cursor
    const hit = painterRef.current?.pickFloor(...ndcOf(e))
    if (!hit) return
    const dx = drag.grabX - dataX(hit.x)
    const dy = drag.grabY - dataY(hit.z)
    setWin((w) => {
      const next = { a: w.a + dx, b: w.b + dx, c: w.c + dy, d: w.d + dy }
      requestResample(next)
      return next
    })
  }

  const onPointerUp = (e: React.PointerEvent) => {
    if (dragRef.current?.pointerId === e.pointerId) dragRef.current = null
  }

  // -- wheel: zoom the domain about the cursor (alt: dolly the camera) -------
  useEffect(() => {
    const el = frameRef.current!
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const factor = Math.exp(e.deltaY * 0.0015)
      if (e.altKey) {
        setOrbit((o) => clampOrbit({ ...o, radius: o.radius * factor }))
        return
      }
      const rect = el.getBoundingClientRect()
      const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1
      const ndcY = -(((e.clientY - rect.top) / rect.height) * 2 - 1)
      const painter = painterRef.current
      // the probe is only recomputed on pointer move — a stale readout must
      // not survive into the new domain
      setProbe(null)
      // zoom about the surface point under the cursor; the floor where the
      // surface has gaps; the window center as a last resort
      const hit = painter?.pick(ndcX, ndcY) ?? painter?.pickFloor(ndcX, ndcY)
      setWin((w) => {
        const cx = hit ? w.a + ((hit.x + 1) / 2) * (w.b - w.a) : (w.a + w.b) / 2
        const cy = hit ? w.c + ((1 - hit.z) / 2) * (w.d - w.c) : (w.c + w.d) / 2
        const next = {
          a: cx - (cx - w.a) * factor,
          b: cx + (w.b - cx) * factor,
          c: cy - (cy - w.c) * factor,
          d: cy + (w.d - cy) * factor,
        }
        const spanX = next.b - next.a
        const spanY = next.d - next.c
        if (spanX < 1e-12 || spanX > 1e12 || spanY < 1e-12 || spanY > 1e12)
          return w
        requestResample(next)
        return next
      })
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
    // requestResample/painterRef are stable refs; win is read via setWin
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plot])

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

  // The composite PNG export: the WebGL frame with the theme background and
  // the optional title (the 3D box carries no 2D ticks, so those gutters
  // collapse). Ref'd so the PDF registration stays stable (see PlotView).
  const buildExportPng = () => {
    const painter = painterRef.current
    if (!painter) return Promise.reject(new Error('plot is not mounted'))
    return exportPlotPng({
      drawFrame: (ctx, x, y, w, h) => painter.drawInto(ctx, x, y, w, h),
      width: size.w,
      height: size.h,
      win: { a: 0, b: 1 },
      yWin: [0, 1],
      xTicks: [],
      yTicks: [],
      title: plot.title,
    })
  }
  const exportRef = useRef(buildExportPng)
  useEffect(() => {
    exportRef.current = buildExportPng
  })

  // Expose this live view to the PDF exporter as a PNG source (see above).
  useEffect(() => {
    if (!cellId) return
    return registerPlotSnapshot(cellId, () => exportRef.current())
  }, [cellId])

  useEffect(() => {
    painterRef.current?.setData(
      surf.heights,
      surf.nx,
      surf.ny,
      zlo,
      zhi,
      sceneScatter,
    )
  }, [surf, zlo, zhi, themeKey, sceneScatter])

  // Pan/zoom updates `win` immediately; the heights catch up when the
  // resample lands. Until then the stale mesh (built over surf.win) is slid/
  // scaled to where its data belongs in the new window — identity when the
  // two windows agree. Declared after the setData effect: a fresh grid must
  // be placed before it paints.
  useEffect(() => {
    const sw = surf.win
    painterRef.current?.setView(
      (sw.b - sw.a) / (win.b - win.a),
      (sw.d - sw.c) / (win.d - win.c),
      -1 + (sw.a + sw.b - 2 * win.a) / (win.b - win.a),
      1 - (sw.c + sw.d - 2 * win.c) / (win.d - win.c),
    )
  }, [win, surf.win])

  useEffect(() => {
    painterRef.current?.setOrbit(orbit)
  }, [orbit])

  // Draw style (solid / glass / wire) is a persisted display preference; the
  // painter updates its material in place, and a later setData rebuild re-reads
  // it, so a resample keeps the chosen look.
  useEffect(() => {
    painterRef.current?.setRenderMode(surfaceRender)
  }, [surfaceRender])

  const reset = () => {
    setOrbit(DEFAULT_ORBIT)
    setWin({ a: plot.a, b: plot.b, c: plot.c, d: plot.d })
    setSurf(sampledOf(plot))
    latestWinRef.current = null
  }

  const savePng = () => {
    void exportRef
      .current()
      .then((png) => saveDataUrl(`${plot.text.slice(0, 40)}.png`, png))
      .catch((e) => console.error('plot export failed', e))
  }

  return (
    <div className="max-w-2xl">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
        <MathInline latex={plot.latex} fallback={plot.text} />
        <span className="text-xs">
          <MathInline latex={nameToLatex(plot.xvar)} fallback={plot.xvar} /> ∈ [
          {formatTick(win.a)}, {formatTick(win.b)}]
        </span>
        <span className="text-xs">
          <MathInline latex={nameToLatex(plot.yvar)} fallback={plot.yvar} /> ∈ [
          {formatTick(win.c)}, {formatTick(win.d)}]
        </span>
        <span
          className="text-xs"
          title="2%–98% quantile range; spikes clamp to the box"
        >
          z ∈ [{formatTick(zlo)}, {formatTick(zhi)}]
        </span>
        {surf.undersampled && (
          <span
            className="text-xs text-warn"
            title={`parts of this window oscillate faster than the finest sampling grid (${surf.nx}×${surf.ny}) can resolve — fine structure may be aliased; zoom in for a faithful view`}
          >
            ⚠ undersampled
          </span>
        )}
        {!pointsOnly && (
          <div className="ml-auto inline-flex overflow-hidden rounded-md border border-edge-strong text-xs">
            {RENDER_MODES.map((m) => (
              <button
                key={m.id}
                onClick={() => setSurfaceRender(m.id)}
                title={m.title}
                aria-pressed={surfaceRender === m.id}
                className={`px-2 ${
                  surfaceRender === m.id
                    ? 'bg-accent/15 text-accent'
                    : 'text-muted hover:text-ink'
                }`}
              >
                {m.label}
              </button>
            ))}
          </div>
        )}
        <button
          onClick={savePng}
          title="download the plot as a PNG"
          className={`${pointsOnly ? 'ml-auto ' : ''}rounded-md border border-edge-strong px-2 text-xs text-muted hover:text-ink`}
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
      {plot.title && (
        <div className="mb-1 text-center text-sm text-ink">
          <MathText text={plot.title} />
        </div>
      )}
      <div
        ref={frameRef}
        title="drag rotates · shift+drag pans · wheel zooms the domain · alt+wheel dollies"
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
            ...(pointsOnly
              ? []
              : [
                  'divider' as const,
                  ...RENDER_MODES.map((m) => ({
                    label: `${surfaceRender === m.id ? '✓ ' : ''}${m.title}`,
                    onSelect: () => setSurfaceRender(m.id),
                  })),
                ]),
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
              className={`pointer-events-none absolute -translate-x-1/2 -translate-y-1/2 ${
                l.isName
                  ? 'text-[11px] text-muted'
                  : 'font-mono text-[10px] text-faint'
              }`}
              style={{ left: positions[i].x, top: positions[i].y }}
            >
              {l.isName ? (
                <MathInline latex={nameToLatex(l.text)} fallback={l.text} />
              ) : (
                l.text
              )}
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
                transform:
                  probePos.x > size.w - 180 ? 'translateX(-100%)' : undefined,
              }}
            >
              ({formatTick(probe.x)}, {formatTick(probe.y)},{' '}
              {formatTick(probe.z)})
            </div>
          </>
        )}
      </div>
    </div>
  )
}
