// React wrapper around LinePlot: owns the view window and samples, wires
// pan/zoom to engine resampling (debounced), and renders DOM tick labels
// positioned by the same scale math the painter uses. Handles any number of
// curves over one shared window; the legend chips use the same palette
// tokens as the painter. The live window readout overlays the frame rather
// than sitting in the header: its width changes every pan/zoom frame, and in
// flow layout that re-wraps the header and bounces the figure.
//
// Y-axis modes: `auto` re-fits the y-window (quantiles) on every resample;
// touching the y-axis (vertical pan, shift+wheel zoom) switches to `manual`,
// which holds the window until reset. Plain wheel zooms x about the cursor.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  normalizePlotData,
  type PlotData,
  type SamplePoint,
} from '../engine/types'
import { useSettings } from '../state/settings'
import { useNotebook } from '../state/store'
import { openContextMenu } from '../state/contextMenu'
import { MathInline } from '../components/MathOutput'
import { LinePlot, seriesColorToken } from './LinePlot'
import { formatTick, niceTicks, quantileDomain } from './scales'

const RESAMPLE_DEBOUNCE_MS = 180

export function PlotView({ plot: rawPlot }: { plot: PlotData }) {
  // Pre-multi-curve persisted results normalize to a one-series shape.
  const plot = useMemo(() => normalizePlotData(rawPlot), [rawPlot])
  const resample = useNotebook((s) => s.resample)
  // The painter samples theme CSS variables when it (re)builds materials;
  // keying the draw effects on the theme makes a mode/accent switch repaint.
  const themeKey = useSettings((s) => `${s.resolvedMode}/${s.accent}`)

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const painterRef = useRef<LinePlot | null>(null)
  const debounceRef = useRef<number>(0)
  const dragRef = useRef<{
    pointerId: number
    lastX: number
    lastY: number
  } | null>(null)
  const yManualRef = useRef(false)

  const initialPoints = useMemo(() => plot.series.map((s) => s.points), [plot])
  const [points, setPoints] = useState<SamplePoint[][]>(initialPoints)
  /** Per-series honesty flags (see PlotSeries.undersampled), updated with
   * every resample. */
  const [undersampled, setUndersampled] = useState<boolean[]>(() =>
    plot.series.map((s) => s.undersampled ?? false),
  )
  const [win, setWin] = useState({ a: plot.a, b: plot.b })
  const [yWin, setYWin] = useState<[number, number]>(() =>
    quantileDomain(initialPoints.flat()),
  )
  const [yManual, setYManual] = useState(false)
  const [size, setSize] = useState({ w: 640, h: 320 })
  /** Measurement cursor: the snapped sample under the pointer (data coords,
   * so it stays glued to the curve through pan/zoom). */
  const [probe, setProbe] = useState<{
    x: number
    y: number
    si: number
  } | null>(null)

  const xTicks = useMemo(() => niceTicks(win.a, win.b), [win])
  const yTicks = useMemo(() => niceTicks(yWin[0], yWin[1]), [yWin])

  const markYManual = () => {
    yManualRef.current = true
    setYManual(true)
  }

  // -- painter lifecycle ----------------------------------------------------
  useEffect(() => {
    const painter = new LinePlot(canvasRef.current!)
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
    painterRef.current?.setData(points)
  }, [points, themeKey])

  useEffect(() => {
    painterRef.current?.setView(
      { a: win.a, b: win.b, lo: yWin[0], hi: yWin[1] },
      xTicks,
      yTicks,
    )
  }, [win, yWin, xTicks, yTicks, themeKey])

  // -- pan / zoom → debounced engine resample -------------------------------
  const requestResample = useCallback(
    (a: number, b: number) => {
      window.clearTimeout(debounceRef.current)
      debounceRef.current = window.setTimeout(() => {
        Promise.all(plot.series.map((s) => resample(s.text, plot.var, a, b)))
          .then((all) => {
            setPoints(all.map((r) => r.points))
            setUndersampled(all.map((r) => r.undersampled))
            if (!yManualRef.current)
              setYWin(quantileDomain(all.flatMap((r) => r.points)))
          })
          .catch(() => {
            // engine busy or restarted — stale samples stay visible, the next
            // interaction tries again
          })
      }, RESAMPLE_DEBOUNCE_MS)
    },
    [plot, resample],
  )

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return // right-click opens the context menu, not a drag
    dragRef.current = {
      pointerId: e.pointerId,
      lastX: e.clientX,
      lastY: e.clientY,
    }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  /** Snap the pointer to the nearest curve sample: index by x (samples are
   * evenly spaced), then pick the series whose value is closest on screen. */
  const PROBE_SNAP_PX = 48
  const updateProbe = (e: React.PointerEvent) => {
    const rect = frameRef.current!.getBoundingClientRect()
    const px = e.clientX - rect.left
    const py = e.clientY - rect.top
    const xData = win.a + (px / size.w) * (win.b - win.a)
    const candidates = points.flatMap((pts, si) => {
      if (pts.length < 2) return []
      const span = pts[pts.length - 1][0] - pts[0][0]
      const idx = Math.round(((xData - pts[0][0]) / span) * (pts.length - 1))
      const [xs, ys] = pts[Math.min(pts.length - 1, Math.max(0, idx))]
      if (ys === null) return []
      const dPx = Math.abs(
        size.h - ((ys - yWin[0]) / (yWin[1] - yWin[0])) * size.h - py,
      )
      return [{ x: xs, y: ys, si, dPx }]
    })
    const best = candidates.reduce(
      (acc, c) => (acc === null || c.dPx < acc.dPx ? c : acc),
      null as (typeof candidates)[number] | null,
    )
    setProbe(best !== null && best.dPx <= PROBE_SNAP_PX ? best : null)
  }

  const onPointerMove = (e: React.PointerEvent) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== e.pointerId) {
      updateProbe(e)
      return
    }
    setProbe(null)
    const dxPx = e.clientX - drag.lastX
    const dyPx = e.clientY - drag.lastY
    drag.lastX = e.clientX
    drag.lastY = e.clientY
    if (dxPx !== 0) {
      setWin((w) => {
        const dx = (-dxPx * (w.b - w.a)) / size.w
        const next = { a: w.a + dx, b: w.b + dx }
        requestResample(next.a, next.b)
        return next
      })
    }
    if (dyPx !== 0) {
      // Screen y grows downward: dragging down moves the window up the data.
      markYManual()
      setYWin(([lo, hi]) => {
        const dy = (dyPx * (hi - lo)) / size.h
        return [lo + dy, hi + dy]
      })
    }
  }

  const onPointerUp = (e: React.PointerEvent) => {
    if (dragRef.current?.pointerId === e.pointerId) dragRef.current = null
  }

  // Wheel must be a manual listener: React's onWheel is passive, and zooming
  // a plot must not also scroll the notebook. Plain wheel zooms x about the
  // cursor; shift+wheel zooms y about the cursor.
  useEffect(() => {
    const el = frameRef.current!
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const rect = el.getBoundingClientRect()
      const delta = e.deltaY !== 0 ? e.deltaY : e.deltaX // shift+wheel is deltaX on macOS
      const factor = Math.exp(delta * 0.0015)
      if (e.shiftKey) {
        const frac = 1 - (e.clientY - rect.top) / rect.height
        markYManual()
        setYWin(([lo, hi]) => {
          const cursorY = lo + frac * (hi - lo)
          const next: [number, number] = [
            cursorY - (cursorY - lo) * factor,
            cursorY + (hi - cursorY) * factor,
          ]
          return next[1] - next[0] < 1e-12 || next[1] - next[0] > 1e12
            ? [lo, hi]
            : next
        })
      } else {
        const frac = (e.clientX - rect.left) / rect.width
        setWin((w) => {
          const cursorX = w.a + frac * (w.b - w.a)
          const next = {
            a: cursorX - (cursorX - w.a) * factor,
            b: cursorX + (w.b - cursorX) * factor,
          }
          if (next.b - next.a < 1e-12 || next.b - next.a > 1e12) return w
          requestResample(next.a, next.b)
          return next
        })
      }
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [requestResample])

  const reset = () => {
    yManualRef.current = false
    setYManual(false)
    setWin({ a: plot.a, b: plot.b })
    setPoints(initialPoints)
    setUndersampled(plot.series.map((s) => s.undersampled ?? false))
    setYWin(quantileDomain(initialPoints.flat()))
  }

  const exprText = plot.series.map((s) => s.text).join(', ')

  const savePng = () => {
    const painter = painterRef.current
    if (!painter) return
    const a = document.createElement('a')
    a.href = painter.snapshot()
    a.download = `${exprText.slice(0, 40)}.png`
    a.click()
  }

  // -- tick label positions (same scales as the painter) ---------------------
  const xPos = (x: number) => ((x - win.a) / (win.b - win.a)) * size.w
  const yPos = (y: number) =>
    size.h - ((y - yWin[0]) / (yWin[1] - yWin[0])) * size.h

  return (
    <div className="max-w-2xl">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
        {plot.series.map((s, i) => (
          <span key={i} className="inline-flex items-baseline gap-1.5">
            <span
              className="inline-block h-2 w-2 self-center rounded-full"
              style={{ background: `var(${seriesColorToken(i)})` }}
            />
            <MathInline latex={s.latex} fallback={s.text} />
          </span>
        ))}
        {undersampled.some(Boolean) && (
          <span
            className="text-xs text-warn"
            title="parts of this window oscillate faster than the finest sampling resolution can resolve — fine structure may be aliased; zoom in for a faithful view"
          >
            ⚠ undersampled
          </span>
        )}
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
        title="drag to pan · wheel zooms x · shift+wheel zooms y"
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
              onSelect: () => void navigator.clipboard.writeText(exprText),
            },
          ])
        }
      >
        <canvas ref={canvasRef} className="block h-full w-full" />
        <div className="pointer-events-none absolute right-1.5 top-1 text-right font-mono text-[10px] leading-4 text-faint">
          <div>
            {plot.var} ∈ [{formatTick(win.a)}, {formatTick(win.b)}]
          </div>
          <div>
            y ∈ [{formatTick(yWin[0])}, {formatTick(yWin[1])}]{' '}
            <span className={yManual ? 'text-warn/80' : ''}>
              {yManual ? 'manual' : 'auto'}
            </span>
          </div>
        </div>
        {xTicks.map((t) => (
          <span
            key={`x${t}`}
            className="pointer-events-none absolute bottom-0.5 -translate-x-1/2 font-mono text-[10px] text-faint"
            style={{ left: xPos(t) }}
          >
            {formatTick(t)}
          </span>
        ))}
        {yTicks.map((t) => (
          <span
            key={`y${t}`}
            className="pointer-events-none absolute left-1 -translate-y-1/2 font-mono text-[10px] text-faint"
            style={{ top: yPos(t) }}
          >
            {formatTick(t)}
          </span>
        ))}
        {probe && (
          <>
            <div
              className="pointer-events-none absolute bottom-0 top-0 w-px bg-edge-strong/70"
              style={{ left: xPos(probe.x) }}
            />
            <div
              className="pointer-events-none absolute left-0 right-0 h-px bg-edge-strong/70"
              style={{ top: yPos(probe.y) }}
            />
            <div
              className="pointer-events-none absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 bg-app"
              style={{
                left: xPos(probe.x),
                top: yPos(probe.y),
                borderColor: `var(${seriesColorToken(probe.si)})`,
              }}
            />
            <div
              className="pointer-events-none absolute z-10 whitespace-nowrap rounded-md border border-edge bg-raised px-1.5 py-0.5 font-mono text-[11px] text-ink"
              style={{
                left: xPos(probe.x) + (xPos(probe.x) > size.w - 130 ? -10 : 10),
                top: yPos(probe.y) - 26,
                transform:
                  xPos(probe.x) > size.w - 130
                    ? 'translateX(-100%)'
                    : undefined,
              }}
            >
              ({formatTick(probe.x)}, {formatTick(probe.y)})
            </div>
          </>
        )}
      </div>
    </div>
  )
}
