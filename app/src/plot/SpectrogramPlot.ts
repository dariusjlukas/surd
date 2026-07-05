// Framework-free 2D-canvas painter for a spectrogram (STFT heatmap). Like
// SplomPlot this draws everything — the heat image, axes, and colorbar —
// into one 2D canvas, so PNG/PDF snapshots are complete and labels stay
// crisp. Static (no pan/zoom): a spectrogram is read, not manipulated.
//
// Color: magnitude is a SEQUENTIAL job, so one luminance-monotone ramp,
// dark = quiet → bright = loud (the inferno ramp — lightness carries the
// signal, which also keeps it CVD-safe). The ramp is fixed across themes —
// the panel is the data image edge-to-edge, so the surface never competes
// with it — while axes, labels, and the frame follow theme tokens.

import { formatTick } from './scales'
import type { SpectrogramData } from '../engine/types'

/** Inferno, 9 stops — perceptually ordered, luminance-monotone. */
const RAMP: [number, number, number][] = [
  [0, 0, 4],
  [27, 12, 65],
  [74, 12, 107],
  [120, 28, 109],
  [165, 44, 96],
  [207, 68, 70],
  [237, 105, 37],
  [251, 155, 6],
  [252, 255, 164],
]

function rampColor(t: number): [number, number, number] {
  const x = Math.min(1, Math.max(0, t)) * (RAMP.length - 1)
  const i = Math.min(RAMP.length - 2, Math.floor(x))
  const f = x - i
  const [r0, g0, b0] = RAMP[i]
  const [r1, g1, b1] = RAMP[i + 1]
  return [r0 + f * (r1 - r0), g0 + f * (g1 - g0), b0 + f * (b1 - b0)]
}

/** A theme token as a CSS color string (index.css guarantees plain hex/rgb). */
function cssColor(token: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim()
  return v || fallback
}

const MARGIN = { top: 8, right: 64, bottom: 26, left: 46 }
const BAR_W = 10
const FONT = '10px ui-sans-serif, system-ui, sans-serif'

export class SpectrogramPlot {
  private canvas: HTMLCanvasElement
  private ctx: CanvasRenderingContext2D
  private w = 1
  private h = 1
  private data: SpectrogramData | null = null
  /** The heat image at data resolution, rebuilt when the data changes. */
  private image: HTMLCanvasElement | null = null

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    this.ctx = canvas.getContext('2d')!
  }

  resize(w: number, h: number) {
    const dpr = window.devicePixelRatio || 1
    this.w = w
    this.h = h
    this.canvas.width = Math.round(w * dpr)
    this.canvas.height = Math.round(h * dpr)
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    this.render()
  }

  setData(d: SpectrogramData) {
    this.data = d
    this.image = this.buildImage(d)
    this.render()
  }

  snapshot(): string {
    this.render()
    return this.canvas.toDataURL('image/png')
  }

  dispose() {}

  /** The dB grid as an offscreen canvas, one pixel per (frame, bin):
   * x = time (frame), y = frequency (bin 0 at the BOTTOM row). */
  private buildImage(d: SpectrogramData): HTMLCanvasElement {
    const off = document.createElement('canvas')
    off.width = d.frames
    off.height = d.bins
    const octx = off.getContext('2d')!
    const img = octx.createImageData(d.frames, d.bins)
    const span = Math.max(1e-9, d.db_max - d.db_min)
    for (let f = 0; f < d.frames; f++) {
      for (let b = 0; b < d.bins; b++) {
        const db = d.db10[f * d.bins + b] / 10
        const [r, g, bl] = rampColor((db - d.db_min) / span)
        // Flip vertically: bin 0 (lowest frequency) at the bottom.
        const px = ((d.bins - 1 - b) * d.frames + f) * 4
        img.data[px] = r
        img.data[px + 1] = g
        img.data[px + 2] = bl
        img.data[px + 3] = 255
      }
    }
    octx.putImageData(img, 0, 0)
    return off
  }

  private render() {
    const { ctx, w, h, data: d } = this
    ctx.clearRect(0, 0, w, h)
    if (!d || !this.image) return
    const ink = cssColor('--ink', '#e5e7eb')
    const faint = cssColor('--faint', '#64748b')
    const grid = cssColor('--plot-grid', '#27303f')

    const px = MARGIN.left
    const py = MARGIN.top
    const pw = Math.max(1, w - MARGIN.left - MARGIN.right)
    const ph = Math.max(1, h - MARGIN.top - MARGIN.bottom)

    // The heat image, scaled to the panel (smoothed — the data grid is
    // usually coarser than the pixels).
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(this.image, px, py, pw, ph)
    ctx.strokeStyle = grid
    ctx.lineWidth = 1
    ctx.strokeRect(px + 0.5, py + 0.5, pw - 1, ph - 1)

    ctx.font = FONT
    ctx.fillStyle = faint

    // Time axis (samples), 4–6 ticks.
    ctx.textAlign = 'center'
    ctx.textBaseline = 'top'
    const tTicks = 5
    for (let i = 0; i <= tTicks; i++) {
      const t = d.t_lo + ((d.t_hi - d.t_lo) * i) / tTicks
      const x = px + (pw * i) / tTicks
      ctx.fillText(formatTick(t), x, py + ph + 4)
    }
    ctx.fillStyle = ink
    ctx.fillText('time (samples)', px + pw / 2, py + ph + 14)

    // Frequency axis, in units of π rad/sample.
    ctx.fillStyle = faint
    ctx.textAlign = 'right'
    ctx.textBaseline = 'middle'
    const fTicks = 4
    for (let i = 0; i <= fTicks; i++) {
      const fv = d.f_lo + ((d.f_hi - d.f_lo) * i) / fTicks
      const y = py + ph - (ph * i) / fTicks
      const label = fv === 0 ? '0' : `${formatTick(fv)}π`
      ctx.fillText(label, px - 5, y)
    }
    ctx.save()
    ctx.translate(10, py + ph / 2)
    ctx.rotate(-Math.PI / 2)
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillStyle = ink
    ctx.fillText('frequency (rad/sample)', 0, 0)
    ctx.restore()

    // Colorbar with dB labels.
    const bx = px + pw + 12
    for (let y = 0; y < ph; y++) {
      const t = 1 - y / Math.max(1, ph - 1)
      const [r, g, b] = rampColor(t)
      ctx.fillStyle = `rgb(${r | 0},${g | 0},${b | 0})`
      ctx.fillRect(bx, py + y, BAR_W, 1)
    }
    ctx.strokeStyle = grid
    ctx.strokeRect(bx + 0.5, py + 0.5, BAR_W - 1, ph - 1)
    ctx.fillStyle = faint
    ctx.textAlign = 'left'
    ctx.textBaseline = 'middle'
    const dbTicks = 3
    for (let i = 0; i <= dbTicks; i++) {
      const db = d.db_min + ((d.db_max - d.db_min) * i) / dbTicks
      const y = py + ph - (ph * i) / dbTicks
      ctx.fillText(`${Math.round(db)} dB`, bx + BAR_W + 4, y)
    }
  }
}
