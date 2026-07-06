// Composite PNG export for plots. The live painters are bare WebGL frames —
// gridlines and curves only; ticks, legend, and labels are DOM overlays that
// a canvas readback never sees. This module rebuilds the full figure on a 2D
// canvas: theme background, the WebGL frame, tick numbers, and the optional
// title / axis labels (KaTeX, rasterized by plot/mathtext — with a plain-text
// fallback so an export never fails outright over label rendering).

import { formatTick } from './scales'
import { mathTextPlain, rasterizeMathText, type LabelImage } from './mathtext'

export interface PlotPngSpec {
  /** Draw the live frame into the composite. Called synchronously in the
   * same task as the final encode (the WebGL backbuffer rule). */
  drawFrame: (
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    w: number,
    h: number,
  ) => void
  /** CSS-pixel size of the live frame; the exported plot area matches it. */
  width: number
  height: number
  /** Ticks and the window that positions them. Empty tick arrays (the 3D
   * view) skip the tick gutters entirely. */
  win: { a: number; b: number }
  yWin: [number, number]
  xTicks: number[]
  yTicks: number[]
  title?: string
  xlabel?: string
  ylabel?: string
  /** Legend entries; pass when curve identification is needed (≥ 2 series).
   * `latex` is engine LaTeX (not mathtext); `color` is a CSS color. */
  legend?: { latex: string; color: string }[]
}

const TICK_FONT = '10px ui-monospace, SFMono-Regular, Menlo, monospace'
const TITLE_PX = 15
const LABEL_PX = 12
const PAD = 10

/** A theme token as a CSS color string (index.css guarantees plain hex). */
function cssColor(token: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim()
  return v || fallback
}

/** A rasterized label, or the plain-text fallback when rasterization is
 * unavailable; `w`/`h` are CSS pixels either way. */
interface Drawn {
  image: LabelImage | null
  text: string
  fontPx: number
  color: string
  w: number
  h: number
}

async function prepare(
  measure: CanvasRenderingContext2D,
  text: string | undefined,
  color: string,
  fontPx: number,
  scale: number,
): Promise<Drawn | null> {
  if (!text) return null
  const image = await rasterizeMathText(text, color, fontPx, scale)
  if (image) return { image, text, fontPx, color, w: image.w, h: image.h }
  const plain = mathTextPlain(text)
  measure.font = `${fontPx}px ui-sans-serif, system-ui, sans-serif`
  return {
    image: null,
    text: plain,
    fontPx,
    color,
    w: Math.ceil(measure.measureText(plain).width),
    h: Math.ceil(fontPx * 1.4),
  }
}

/** Draw a prepared label with its top-left corner at (x, y). */
function drawLabel(
  ctx: CanvasRenderingContext2D,
  l: Drawn,
  x: number,
  y: number,
) {
  if (l.image) {
    ctx.drawImage(l.image.img, x, y, l.w, l.h)
  } else {
    ctx.font = `${l.fontPx}px ui-sans-serif, system-ui, sans-serif`
    ctx.fillStyle = l.color
    ctx.textAlign = 'left'
    ctx.textBaseline = 'top'
    ctx.fillText(l.text, x, y + (l.h - l.fontPx) / 2)
  }
}

/** Render the full figure and return it as a PNG data URL. */
export async function exportPlotPng(spec: PlotPngSpec): Promise<string> {
  const scale = Math.max(2, window.devicePixelRatio || 1)
  const ink = cssColor('--ink', '#e5e7eb')
  const muted = cssColor('--muted', '#94a3b8')
  const faint = cssColor('--faint', '#64748b')
  const surface = cssColor('--surface', '#0f172a')
  const edge = cssColor('--edge', '#1e293b')

  const measure = document.createElement('canvas').getContext('2d')!
  const [title, xlabel, ylabel, legend] = await Promise.all([
    prepare(measure, spec.title, ink, TITLE_PX, scale),
    prepare(measure, spec.xlabel, muted, LABEL_PX, scale),
    prepare(measure, spec.ylabel, muted, LABEL_PX, scale),
    Promise.all(
      (spec.legend ?? []).map(async (entry) => ({
        color: entry.color,
        label: await prepare(measure, `$${entry.latex}$`, ink, LABEL_PX, scale),
      })),
    ),
  ])

  // -- gutters ---------------------------------------------------------------
  measure.font = TICK_FONT
  const yTickW = spec.yTicks.length
    ? Math.ceil(
        Math.max(
          ...spec.yTicks.map((t) => measure.measureText(formatTick(t)).width),
        ),
      ) + 8
    : 0
  const xTickH = spec.xTicks.length ? 16 : 0
  // The rotated y-label occupies its (text) height as horizontal width.
  const left = PAD + (ylabel ? ylabel.h + 4 : 0) + yTickW
  const right = PAD + 6
  const titleH = title ? title.h + 6 : 0

  // Legend chips wrap into rows sized to the plot width.
  const CHIP_DOT = 7
  const CHIP_GAP = 16
  const chips = legend.filter((c) => c.label !== null)
  const rows: (typeof chips)[] = []
  {
    let row: typeof chips = []
    let x = 0
    for (const chip of chips) {
      const w = CHIP_DOT + 5 + chip.label!.w
      if (row.length && x + w > spec.width) {
        rows.push(row)
        row = []
        x = 0
      }
      row.push(chip)
      x += w + CHIP_GAP
    }
    if (row.length) rows.push(row)
  }
  const rowH = chips.length ? Math.max(...chips.map((c) => c.label!.h)) + 4 : 0
  const legendH = rows.length * rowH + (rows.length ? 4 : 0)
  const top = PAD + titleH + legendH
  const bottom = xTickH + (xlabel ? xlabel.h + 4 : 0) + PAD

  const W = left + spec.width + right
  const H = top + spec.height + bottom

  const canvas = document.createElement('canvas')
  canvas.width = Math.round(W * scale)
  canvas.height = Math.round(H * scale)
  const ctx = canvas.getContext('2d')!
  ctx.scale(scale, scale)

  // -- background, frame, border ----------------------------------------------
  ctx.fillStyle = surface
  ctx.fillRect(0, 0, W, H)
  spec.drawFrame(ctx, left, top, spec.width, spec.height)
  ctx.strokeStyle = edge
  ctx.lineWidth = 1
  ctx.strokeRect(left + 0.5, top + 0.5, spec.width - 1, spec.height - 1)

  // -- ticks -------------------------------------------------------------------
  const { a, b } = spec.win
  const [lo, hi] = spec.yWin
  const xPos = (x: number) => left + ((x - a) / (b - a)) * spec.width
  const yPos = (y: number) =>
    top + spec.height - ((y - lo) / (hi - lo)) * spec.height
  ctx.font = TICK_FONT
  ctx.fillStyle = faint
  ctx.textAlign = 'center'
  ctx.textBaseline = 'top'
  for (const t of spec.xTicks)
    ctx.fillText(formatTick(t), xPos(t), top + spec.height + 5)
  ctx.textAlign = 'right'
  ctx.textBaseline = 'middle'
  for (const t of spec.yTicks) ctx.fillText(formatTick(t), left - 5, yPos(t))

  // -- title, legend, labels ----------------------------------------------------
  if (title) drawLabel(ctx, title, left + (spec.width - title.w) / 2, PAD)
  rows.forEach((row, ri) => {
    const rowW = row.reduce(
      (acc, c) => acc + CHIP_DOT + 5 + c.label!.w + CHIP_GAP,
      -CHIP_GAP,
    )
    let x = left + Math.max(0, (spec.width - rowW) / 2)
    const y = PAD + titleH + ri * rowH
    for (const chip of row) {
      ctx.fillStyle = chip.color
      ctx.beginPath()
      ctx.arc(x + CHIP_DOT / 2, y + rowH / 2 - 2, CHIP_DOT / 2, 0, Math.PI * 2)
      ctx.fill()
      drawLabel(
        ctx,
        chip.label!,
        x + CHIP_DOT + 5,
        y + (rowH - 4 - chip.label!.h) / 2,
      )
      x += CHIP_DOT + 5 + chip.label!.w + CHIP_GAP
    }
  })
  if (xlabel)
    drawLabel(
      ctx,
      xlabel,
      left + (spec.width - xlabel.w) / 2,
      top + spec.height + xTickH + 2,
    )
  if (ylabel) {
    ctx.save()
    ctx.translate(PAD + ylabel.h / 2, top + spec.height / 2)
    ctx.rotate(-Math.PI / 2)
    drawLabel(ctx, ylabel, -ylabel.w / 2, -ylabel.h / 2)
    ctx.restore()
  }

  return canvas.toDataURL('image/png')
}
