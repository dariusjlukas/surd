// React wrapper around SpectrogramPlot: owns the canvas, keeps it sized to
// the column width (16:9), repaints on theme change, and registers the view
// for PDF export. Static like the SPLOM — the figure, a caption, and a PNG
// export.

import { useEffect, useRef } from 'react'
import type { SpectrogramData } from '../engine/types'
import { useSettings } from '../state/settings'
import { openContextMenu } from '../state/contextMenu'
import { registerPlotSnapshot } from './snapshots'
import { saveDataUrl } from '../platform/desktop'
import { SpectrogramPlot } from './SpectrogramPlot'

export function SpectrogramView({
  spectrogram,
  cellId,
}: {
  spectrogram: SpectrogramData
  cellId?: string
}) {
  const themeKey = useSettings((s) => `${s.resolvedMode}/${s.accent}`)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const frameRef = useRef<HTMLDivElement>(null)
  const painterRef = useRef<SpectrogramPlot | null>(null)

  useEffect(() => {
    const painter = new SpectrogramPlot(canvasRef.current!)
    painterRef.current = painter
    const ro = new ResizeObserver(([entry]) => {
      const w = entry.contentRect.width
      if (w > 0) painter.resize(Math.round(w), Math.round((w * 9) / 16))
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
    painterRef.current?.setData(spectrogram)
  }, [spectrogram, themeKey])

  const savePng = () => {
    const painter = painterRef.current
    if (!painter) return
    void saveDataUrl('spectrogram.png', painter.snapshot()).catch((e) =>
      console.error('plot export failed', e),
    )
  }

  const caption = `${spectrogram.total_frames} frames × ${spectrogram.bins} bins${
    spectrogram.pooled ? ' (display-pooled)' : ''
  }`

  return (
    <div className="max-w-2xl">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
        <span>spectrogram</span>
        <span className="text-xs text-faint">{caption}</span>
        <button
          onClick={savePng}
          title="download the spectrogram as a PNG"
          className="ml-auto rounded-md border border-edge-strong px-2 text-xs text-muted hover:text-ink"
        >
          png
        </button>
      </div>
      <div
        ref={frameRef}
        title="STFT magnitude, dB — Hann window; brighter = louder"
        className="relative aspect-video w-full overflow-hidden rounded-lg border border-edge bg-surface"
        onContextMenu={(e) =>
          openContextMenu(e, [{ label: 'Save as PNG', onSelect: savePng }])
        }
      >
        <canvas ref={canvasRef} className="block h-full w-full" />
      </div>
    </div>
  )
}
