// Markdown → sanitized HTML, with LaTeX math support.
//
// Math is handled in two passes so KaTeX's output never has to survive
// DOMPurify (which can strip the inline styles KaTeX positions glyphs with):
//
//   1. renderMarkdown — a private `marked` instance with a math extension turns
//      `$…$` / `$$…$$` into EMPTY placeholder elements that carry the raw TeX,
//      base64-encoded, in a data attribute. The surrounding prose renders
//      normally, then the whole thing is sanitized. Placeholders (plain
//      spans/divs with a class and a data-* attr) pass DOMPurify untouched.
//   2. hydrateMath — after the sanitized HTML is in the DOM, each placeholder
//      is rendered in place by KaTeX. KaTeX output (trust:false) is XSS-safe,
//      so it goes straight into the DOM without re-sanitizing.
//
// The extension is tokenizer-aware (marked won't see `$` inside code spans or
// fenced blocks), so inline code like `cost is $5` stays literal.

import DOMPurify from 'dompurify'
import katex from 'katex'
import { Marked, type TokenizerAndRendererExtension } from 'marked'

// base64 round-trips arbitrary TeX (incl. unicode) through an HTML attribute
// value without `<`, `>`, `&`, or quotes leaking out.
function encodeTex(s: string): string {
  const bytes = new TextEncoder().encode(s)
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}
function decodeTex(b64: string): string {
  const bin = atob(b64)
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)))
}

/** An empty element carrying TeX for {@link hydrateMath} to render in place.
 * Shared by the markdown extension and the PDF exporter (which feeds math-cell
 * results through the same hydration pass). */
export function mathPlaceholder(tex: string, display: boolean): string {
  const tag = display ? 'div' : 'span'
  return `<${tag} class="surd-math" data-tex="${encodeTex(tex)}" data-display="${display ? '1' : '0'}"></${tag}>`
}

// Inline `$…$`: opener not followed by whitespace, closer not preceded by it,
// and not closed right before a digit — so `$5 and $10` reads as currency, not
// math. Single line only.
const INLINE = /^\$(?![\s$])((?:\\\$|[^$\n])*?)(?<!\s)\$(?!\d)/
// Block `$$…$$`: its own paragraph, may span lines.
const BLOCK = /^ {0,3}\$\$([\s\S]+?)\$\$(?:\n+|$)/

const inlineMath: TokenizerAndRendererExtension = {
  name: 'mathInline',
  level: 'inline',
  start(src) {
    const i = src.indexOf('$')
    return i < 0 ? undefined : i
  },
  tokenizer(src) {
    const m = INLINE.exec(src)
    if (m) return { type: 'mathInline', raw: m[0], text: m[1] }
  },
  renderer(token) {
    return mathPlaceholder(token.text, false)
  },
}

const blockMath: TokenizerAndRendererExtension = {
  name: 'mathBlock',
  level: 'block',
  start(src) {
    const i = src.indexOf('$$')
    return i < 0 ? undefined : i
  },
  tokenizer(src) {
    const m = BLOCK.exec(src)
    if (m) return { type: 'mathBlock', raw: m[0], text: m[1].trim() }
  },
  renderer(token) {
    return mathPlaceholder(token.text, true)
  },
}

const md = new Marked({ extensions: [blockMath, inlineMath] })

/** Markdown source → sanitized HTML with math placeholders left intact. */
export function renderMarkdown(src: string): string {
  return DOMPurify.sanitize(md.parse(src, { async: false }))
}

/** Render every math placeholder under `root` in place with KaTeX. Idempotent:
 * a placeholder loses its `data-tex` once rendered, so a re-run (e.g. React
 * StrictMode's double effect) skips it. */
export function hydrateMath(root: HTMLElement): void {
  root.querySelectorAll<HTMLElement>('.surd-math[data-tex]').forEach((el) => {
    const tex = decodeTex(el.dataset.tex ?? '')
    el.removeAttribute('data-tex')
    try {
      katex.render(tex, el, {
        displayMode: el.dataset.display === '1',
        throwOnError: false,
      })
    } catch {
      el.textContent = tex
    }
  })
}
