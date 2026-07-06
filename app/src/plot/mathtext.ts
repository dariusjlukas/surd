// Matplotlib-style "mathtext" for plot titles and axis labels: plain text
// with `$...$` segments rendered as LaTeX math. Two consumers:
//   * the MathText component (components/MathOutput.tsx) renders it live;
//   * rasterizeMathText turns it into an image for the PNG export path,
//     via an SVG foreignObject carrying the document's KaTeX CSS with the
//     fonts inlined as data: URIs (an SVG image can't fetch anything, so
//     everything must ride along).
//
// Rasterization is best-effort by design: some engines (older WebKit) taint
// a canvas that has drawn a foreignObject SVG, which would break the whole
// export. So each label is first drawn to a throwaway canvas and re-encoded
// as a PNG — if that throws, rasterizeMathText resolves to null and the
// compositor falls back to plain canvas text instead of losing the export.

import katex from 'katex'

export interface MathTextSegment {
  math: boolean
  value: string
}

/** Split into text and `$...$` math segments. `\$` is a literal dollar; an
 * unmatched `$` stays literal text. */
export function splitMathText(text: string): MathTextSegment[] {
  const segs: MathTextSegment[] = []
  let plain = ''
  let i = 0
  const closeOf = (from: number): number => {
    for (let j = from; j < text.length; j++) {
      if (text[j] === '\\')
        j++ // skip the escaped char
      else if (text[j] === '$') return j
    }
    return -1
  }
  while (i < text.length) {
    if (text[i] === '\\' && text[i + 1] === '$') {
      plain += '$'
      i += 2
    } else if (text[i] === '$') {
      const close = closeOf(i + 1)
      if (close === -1) {
        plain += text.slice(i)
        break
      }
      if (plain) {
        segs.push({ math: false, value: plain })
        plain = ''
      }
      segs.push({ math: true, value: text.slice(i + 1, close) })
      i = close + 1
    } else {
      plain += text[i]
      i += 1
    }
  }
  if (plain) segs.push({ math: false, value: plain })
  return segs
}

/** The label without math markup — canvas-text fallback and alt text. */
export function mathTextPlain(text: string): string {
  return splitMathText(text)
    .map((s) => s.value)
    .join('')
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** Mathtext → HTML string for the rasterizer. A math segment KaTeX can't
 * parse falls back to its literal `$...$` text (same honesty rule as
 * MathInline). `output: 'html'` skips the hidden MathML block — this string
 * is embedded in an SVG foreignObject, which is parsed as XML and must stay
 * well-formed. */
export function mathTextHtml(text: string): string {
  return splitMathText(text)
    .map((seg) => {
      if (!seg.math) return escapeHtml(seg.value)
      try {
        return katex.renderToString(seg.value, {
          throwOnError: true,
          output: 'html',
        })
      } catch {
        return escapeHtml(`$${seg.value}$`)
      }
    })
    .join('')
}

// ---------------------------------------------------------------------------
// Rasterization (PNG export path)
// ---------------------------------------------------------------------------

export interface LabelImage {
  /** PNG-backed image, `scale`× the CSS-pixel size below. */
  img: HTMLImageElement
  w: number
  h: number
}

/** KaTeX layout rules + @font-face rules with the fonts inlined as data:
 * URIs, collected once from the document's (bundled, same-origin)
 * stylesheets. */
let katexCssPromise: Promise<string> | null = null

function katexCssInlined(): Promise<string> {
  katexCssPromise ??= buildKatexCss().catch((e) => {
    katexCssPromise = null // transient (font fetch) failures may retry
    throw e
  })
  return katexCssPromise
}

async function buildKatexCss(): Promise<string> {
  const layout: string[] = []
  const fonts: Promise<string>[] = []
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList
    try {
      rules = sheet.cssRules
    } catch {
      continue // cross-origin stylesheet — KaTeX's is bundled, so not it
    }
    for (const rule of Array.from(rules)) {
      if (rule instanceof CSSFontFaceRule) {
        if (rule.style.getPropertyValue('font-family').includes('KaTeX'))
          fonts.push(inlineFontFace(rule, sheet.href))
      } else if (rule.cssText.includes('.katex')) {
        layout.push(rule.cssText)
      }
    }
  }
  const faces = await Promise.all(fonts)
  return [...faces, ...layout].join('\n')
}

async function inlineFontFace(
  rule: CSSFontFaceRule,
  sheetHref: string | null,
): Promise<string> {
  const src = rule.style.getPropertyValue('src')
  const urls = [
    ...src.matchAll(/url\((?:"([^"]+)"|'([^']+)'|([^"')]+))\)/g),
  ].map((m) => m[1] ?? m[2] ?? m[3])
  const chosen = urls.find((u) => u.includes('woff2')) ?? urls[0]
  if (!chosen) return rule.cssText
  const abs = new URL(chosen, sheetHref ?? document.baseURI).href
  const blob = await (await fetch(abs)).blob()
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(r.result as string)
    r.onerror = () => reject(r.error ?? new Error('font read failed'))
    r.readAsDataURL(blob)
  })
  const family = rule.style.getPropertyValue('font-family')
  const style = rule.style.getPropertyValue('font-style') || 'normal'
  const weight = rule.style.getPropertyValue('font-weight') || 'normal'
  return `@font-face{font-family:${family};font-style:${style};font-weight:${weight};src:url(${dataUrl}) format('woff2')}`
}

const rasterCache = new Map<string, Promise<LabelImage | null>>()

/** Rasterize mathtext to a self-contained PNG-backed image, or null when the
 * environment can't do it soundly (foreignObject taint, font fetch failure)
 * — the compositor then draws plain text instead. Results are cached; the
 * color is baked in, so a theme switch is a different key. */
export function rasterizeMathText(
  text: string,
  color: string,
  fontPx: number,
  scale: number,
): Promise<LabelImage | null> {
  const key = `${fontPx}|${scale}|${color}|${text}`
  let hit = rasterCache.get(key)
  if (!hit) {
    hit = doRasterize(text, color, fontPx, scale).catch((e) => {
      // Fallback is by design (plain canvas text), but say why in the console
      // so a broken environment is diagnosable rather than silently uglier.
      console.warn('plot label rasterization failed; using plain text:', e)
      return null
    })
    rasterCache.set(key, hit)
    if (rasterCache.size > 128) {
      // labels are tiny; this is just a leak guard, oldest-first
      const oldest = rasterCache.keys().next().value
      if (oldest !== undefined) rasterCache.delete(oldest)
    }
  }
  return hit
}

async function doRasterize(
  text: string,
  color: string,
  fontPx: number,
  scale: number,
): Promise<LabelImage> {
  const css = await katexCssInlined()
  const style = `display:inline-block;white-space:nowrap;font:${fontPx}px ui-sans-serif,system-ui,sans-serif;color:${color}`
  const html = mathTextHtml(text)

  // Measure in the live document (same fonts, same layout engine).
  const host = document.createElement('div')
  host.style.cssText = `position:fixed;left:-10000px;top:0;${style}`
  host.innerHTML = html
  document.body.appendChild(host)
  const rect = host.getBoundingClientRect()
  host.remove()
  // KaTeX ascenders/descenders can poke past the line box; pad a little.
  const w = Math.ceil(rect.width) + 4
  const h = Math.ceil(rect.height) + 4

  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w * scale}" height="${h * scale}" viewBox="0 0 ${w} ${h}">` +
    `<foreignObject width="100%" height="100%">` +
    `<div xmlns="http://www.w3.org/1999/xhtml" style="${escapeHtml(`padding:2px;${style}`)}">` +
    `<style>${css}</style>${html}` +
    `</div></foreignObject></svg>`

  const img = new Image()
  await new Promise<void>((resolve, reject) => {
    img.onload = () => resolve()
    img.onerror = () => reject(new Error('label SVG failed to rasterize'))
    // A data: URL, deliberately not a blob: URL — Chrome taints canvases
    // that drew a blob-loaded foreignObject SVG, while data: SVGs stay clean.
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
  })
  await img.decode().catch(() => undefined) // best-effort; onload already fired

  // Re-encode through a throwaway canvas. This is the taint probe (toDataURL
  // throws on engines that taint foreignObject SVGs → null fallback) and it
  // makes the cached image an inert PNG that can never taint the composite.
  // WebKit loads fonts inside SVG images lazily, so draw once to warm them,
  // yield a moment, and encode the second draw.
  const tmp = document.createElement('canvas')
  tmp.width = w * scale
  tmp.height = h * scale
  const ctx = tmp.getContext('2d')!
  ctx.drawImage(img, 0, 0)
  await new Promise((r) => setTimeout(r, 30))
  ctx.clearRect(0, 0, tmp.width, tmp.height)
  ctx.drawImage(img, 0, 0)
  const png = tmp.toDataURL('image/png')

  const out = new Image()
  await new Promise<void>((resolve, reject) => {
    out.onload = () => resolve()
    out.onerror = () => reject(new Error('label PNG failed to decode'))
    out.src = png
  })
  return { img: out, w, h }
}
