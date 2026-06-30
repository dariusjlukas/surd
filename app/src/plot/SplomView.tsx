// React wrapper around SplomPlot: owns the square canvas, keeps it sized to
// the column width, repaints on theme change, and registers the view for PDF
// export. A scatterplot matrix is read, not manipulated, so there's no pan/zoom
// here — just the figure, a caption, and a PNG export (matching PlotView).

import { useEffect, useMemo, useRef } from 'react'
import type { SplomData } from '../engine/types'
import { useSettings } from '../state/settings'
import { openContextMenu } from '../state/contextMenu'
import { registerPlotSnapshot } from './snapshots'
import { saveDataUrl } from '../platform/desktop'
import { SplomPlot } from './SplomPlot'

export function SplomView({
  splom,
  cellId,
}: {
  splom: SplomData
  /** Owning cell id, when shown in a notebook cell: registers this live view
   * for PDF export (see plot/snapshots). Absent for previews/tests. */
  cellId?: string
}) {
  // The painter samples theme CSS variables when it draws; keying the repaint
  // effect on the theme makes a mode/accent switch take effect.
  const themeKey = useSettings((s) => `${s.resolvedMode}/${s.accent}`)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const painterRef = useRef<SplomPlot | null>(null)

  const input = useMemo(
    () => ({
      labels: splom.labels,
      columns: splom.columns,
      ranges: splom.ranges,
      cor: splom.cor,
    }),
    [splom],
  )

  // -- painter lifecycle ----------------------------------------------------
  useEffect(() => {
    const painter = new SplomPlot(canvasRef.current!)
    painterRef.current = painter
    const ro = new ResizeObserver(([entry]) => {
      const w = entry.contentRect.width
      if (w > 0) painter.resize(Math.round(w))
    })
    ro.observe(frameRef.current!)
    return () => {
      ro.disconnect()
      painter.dispose()
      painterRef.current = null
    }
  }, [])

  useEffect(() => {
    if (!cellId) return
    return registerPlotSnapshot(
      cellId,
      () => painterRef.current?.snapshot() ?? '',
    )
  }, [cellId])

  useEffect(() => {
    painterRef.current?.setData(input)
  }, [input, themeKey])

  const savePng = () => {
    const painter = painterRef.current
    if (!painter) return
    void saveDataUrl('scatterplot-matrix.png', painter.snapshot()).catch((e) =>
      console.error('plot export failed', e),
    )
  }

  const thinned = splom.shown < splom.total
  const caption = `${splom.labels.length} variables · ${splom.total} observations${
    thinned ? ` (showing ${splom.shown})` : ''
  }`

  return (
    <div className="max-w-2xl">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
        <span>scatterplot matrix</span>
        <span className="text-xs text-faint">{caption}</span>
        <button
          onClick={savePng}
          title="download the matrix as a PNG"
          className="ml-auto rounded-md border border-edge-strong px-2 text-xs text-muted hover:text-ink"
        >
          png
        </button>
      </div>
      <div
        ref={frameRef}
        title="lower triangle: scatter · upper triangle: correlation r · diagonal: variable"
        className="relative aspect-square w-full overflow-hidden rounded-lg border border-edge bg-surface"
        onContextMenu={(e) =>
          openContextMenu(e, [{ label: 'Save as PNG', onSelect: savePng }])
        }
      >
        <canvas ref={canvasRef} className="block h-full w-full" />
      </div>
    </div>
  )
}
