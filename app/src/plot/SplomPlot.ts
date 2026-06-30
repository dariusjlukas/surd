// Framework-free 2D-canvas painter for a scatterplot matrix (SPLOM). A static
// k×k grid: lower triangle scatter, upper triangle correlation, diagonal
// variable names. Unlike the WebGL line/surface painters this draws everything
// — markers, borders, and text — into one 2D canvas, so the PNG/PDF snapshot is
// complete and the labels stay crisp. A SPLOM needs no pan/zoom, so the extra
// machinery of an orthographic camera buys nothing here.

import { formatTick } from './scales'

export interface SplomInput {
  labels: string[]
  /** k columns of samples; null is a gap (drawn as no marker). */
  columns: (number | null)[][]
  /** [min, max] per variable — the shared scale across its row and column. */
  ranges: [number, number][]
  /** Row-major k×k Pearson r; null where undefined. */
  cor: (number | null)[]
}

/** Marker radius in CSS pixels — small, since panels are dense and tiny. */
const MARKER_R = 1.6
/** Marker fill opacity, so overplotting in a cluster reads as density. */
const MARKER_ALPHA = 0.55
/** Inner margin of each panel as a fraction of the cell, so markers and text
 * don't crowd the borders. */
const PAD = 0.08

/** A theme token as a CSS color string (index.css guarantees plain hex/rgb). */
function cssColor(token: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim()
  return v || fallback
}

export class SplomPlot {
  private canvas: HTMLCanvasElement
  private ctx: CanvasRenderingContext2D
  private side = 1
  private data: SplomInput | null = null

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    this.ctx = canvas.getContext('2d')!
  }

  /** Resize to a square of `side` CSS pixels (device-pixel-ratio aware). */
  resize(side: number) {
    const dpr = window.devicePixelRatio || 1
    this.side = side
    this.canvas.width = Math.round(side * dpr)
    this.canvas.height = Math.round(side * dpr)
    // Draw in CSS-pixel coordinates; the transform handles the dpr scale.
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    this.render()
  }

  setData(d: SplomInput) {
    this.data = d
    this.render()
  }

  /** PNG of the whole matrix — markers, borders, and text all included. */
  snapshot(): string {
    this.render()
    return this.canvas.toDataURL('image/png')
  }

  dispose() {}

  private render() {
    const { ctx, side: S, data: d } = this
    ctx.clearRect(0, 0, S, S)
    if (!d) return
    const k = d.labels.length
    if (k < 2) return
    const cell = S / k

    const gridColor = cssColor('--plot-grid', '#27303f')
    const inkColor = cssColor('--ink', '#e5e7eb')
    const accent = cssColor('--accent', '#7dd3fc')
    const negColor = cssColor('--danger', '#f87171')

    // Panel borders.
    ctx.strokeStyle = gridColor
    ctx.lineWidth = 1
    for (let i = 0; i <= k; i++) {
      const p = Math.round(i * cell) + 0.5 // crisp 1px lines
      ctx.beginPath()
      ctx.moveTo(p, 0)
      ctx.lineTo(p, S)
      ctx.moveTo(0, p)
      ctx.lineTo(S, p)
      ctx.stroke()
    }

    // Scatter points (lower triangle: row i below diagonal, i > j). Batched
    // into one path per panel and filled once — many small arcs otherwise.
    ctx.fillStyle = accent
    ctx.globalAlpha = MARKER_ALPHA
    for (let i = 0; i < k; i++) {
      for (let j = 0; j < i; j++) {
        const [xlo, xhi] = d.ranges[j]
        const [ylo, yhi] = d.ranges[i]
        const left = j * cell + PAD * cell
        const top = i * cell + PAD * cell
        const w = cell * (1 - 2 * PAD)
        const h = cell * (1 - 2 * PAD)
        const cx = d.columns[j]
        const cy = d.columns[i]
        ctx.beginPath()
        for (let r = 0; r < cx.length; r++) {
          const xv = cx[r]
          const yv = cy[r]
          if (xv === null || yv === null) continue
          const px = left + ((xv - xlo) / (xhi - xlo)) * w
          // Canvas y grows downward, so the data max sits at the top.
          const py = top + (1 - (yv - ylo) / (yhi - ylo)) * h
          ctx.moveTo(px + MARKER_R, py)
          ctx.arc(px, py, MARKER_R, 0, Math.PI * 2)
        }
        ctx.fill()
      }
    }
    ctx.globalAlpha = 1

    // Diagonal: variable names.
    ctx.fillStyle = inkColor
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    const nameSize = Math.max(10, Math.min(16, cell * 0.18))
    ctx.font = `600 ${nameSize}px ui-sans-serif, system-ui, sans-serif`
    for (let i = 0; i < k; i++) {
      const c = i * cell + cell / 2
      ctx.fillText(fit(ctx, d.labels[i], cell * 0.84), c, c)
    }

    // Upper triangle (i < j): the correlation r, coloured by sign. One
    // consistent size for every panel, sized to fit the cell — the number
    // itself carries the magnitude, so the text doesn't also encode it.
    const corSize = Math.max(10, Math.min(20, cell * 0.22))
    ctx.font = `600 ${corSize}px ui-sans-serif, system-ui, sans-serif`
    const faint = cssColor('--faint', '#64748b')
    for (let i = 0; i < k; i++) {
      for (let j = i + 1; j < k; j++) {
        const r = d.cor[i * k + j]
        const cxp = j * cell + cell / 2
        const cyp = i * cell + cell / 2
        if (r === null) {
          ctx.fillStyle = faint
          ctx.fillText('—', cxp, cyp)
          continue
        }
        ctx.fillStyle = r >= 0 ? accent : negColor
        ctx.fillText(fit(ctx, formatTick(r), cell * 0.84), cxp, cyp)
      }
    }
  }
}

/** Truncate text with an ellipsis to fit `maxWidth` (so long names don't spill
 * across panel borders). */
function fit(
  ctx: CanvasRenderingContext2D,
  text: string,
  maxWidth: number,
): string {
  if (ctx.measureText(text).width <= maxWidth) return text
  let t = text
  while (t.length > 1 && ctx.measureText(t + '…').width > maxWidth)
    t = t.slice(0, -1)
  return t + '…'
}
