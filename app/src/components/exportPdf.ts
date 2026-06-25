// Notebook → PDF, via the browser/webview's own "Save as PDF" print path.
//
// We build a clean, paper-themed report element (#print-report) in the document
// body and call window.print(); a print stylesheet (see index.css) hides the
// live app and swaps the report in. Printing the live notebook directly won't
// do: it carries the app chrome, and — fatally — the WebGL plot canvases read
// back blank under the browser's print snapshot. So every cell is re-rendered
// here for paper, and plots are embedded as the PNG snapshots their live views
// register (see plot/snapshots).
//
// Reusing the in-app renderers keeps the report faithful: markdown goes through
// the same sanitizer, and ALL math — prose `$…$`, math-cell results — is
// emitted as placeholders and rendered by one hydrateMath pass, so it uses the
// KaTeX fonts already loaded in the document.

import type { Cell, Notebook } from '../state/store'
import { hydrateMath, mathPlaceholder, renderMarkdown } from './markdown'
import { plotSnapshot } from '../plot/snapshots'
import { isTauri, printDocument } from '../platform/desktop'

const REPORT_ID = 'print-report'

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** The result block under a math or data cell's source. */
function resultHtml(cell: Cell): string {
  if (cell.status === 'pending')
    return `<div class="report-note">not evaluated</div>`
  if (cell.status === 'cancelled')
    return `<div class="report-note">cancelled</div>`
  const r = cell.result
  if (!r) return ''
  if (!r.ok)
    return `<div class="report-error">error: ${escapeHtml(r.error ?? '')}</div>`
  switch (r.kind) {
    case 'plot':
    case 'plot3d': {
      const png = plotSnapshot(cell.id)
      return png
        ? `<img class="report-plot" src="${png}" alt="${escapeHtml(r.text)}" />`
        : `<div class="report-note">[plot — open this notebook to include it in the export]</div>`
    }
    case 'function':
    case 'data':
      // descriptive text ("<function(n)>", import summaries), not math
      return `<div class="report-text">${escapeHtml(r.text)}</div>`
    default:
      return `<div class="report-result">${mathPlaceholder(r.latex, true)}</div>`
  }
}

function cellHtml(cell: Cell): string {
  if (cell.kind === 'markdown') {
    if (!cell.src.trim()) return ''
    return `<section class="report-cell report-md md-cell">${renderMarkdown(cell.src)}</section>`
  }
  const sigil = cell.kind === 'data' ? '⇣' : '&gt;&gt;'
  return `<section class="report-cell report-${cell.kind}">
    <div class="report-src"><span class="report-sigil">${sigil}</span> ${escapeHtml(cell.src)}</div>
    ${resultHtml(cell)}
  </section>`
}

/** The report's inner HTML: header (title + date) then one block per cell. */
export function buildReportHtml(nb: Notebook): string {
  const date = new Date(nb.updatedAt).toLocaleString(undefined, {
    dateStyle: 'long',
    timeStyle: 'short',
  })
  const body = nb.cells.map(cellHtml).join('\n')
  return `<header class="report-header">
      <h1 class="report-title">${escapeHtml(nb.name)}</h1>
      <div class="report-date">${escapeHtml(date)}</div>
    </header>
    <main class="report-body">${body || '<p class="report-note">This notebook is empty.</p>'}</main>`
}

/** Resolve once every <img> in `root` has loaded (or failed) — print must wait
 * for plot PNGs to decode or they paginate blank. */
function imagesReady(root: HTMLElement): Promise<void> {
  const pending = Array.from(root.querySelectorAll('img')).filter(
    (img) => !img.complete,
  )
  return Promise.all(
    pending.map(
      (img) =>
        new Promise<void>((resolve) => {
          img.addEventListener('load', () => resolve(), { once: true })
          img.addEventListener('error', () => resolve(), { once: true })
        }),
    ),
  ).then(() => undefined)
}

/** Render `nb` into a printable report and open the print dialog ("Save as
 * PDF"). Reuses one #print-report node — a prior, lingering report is replaced
 * rather than duplicated. */
export async function exportNotebookPdf(nb: Notebook): Promise<void> {
  const container =
    document.getElementById(REPORT_ID) ?? document.createElement('div')
  container.id = REPORT_ID
  container.innerHTML = buildReportHtml(nb)
  if (!container.isConnected) document.body.appendChild(container)

  hydrateMath(container)
  await imagesReady(container)

  // Desktop runs a modal native print operation that fires no `afterprint`
  // event, so we leave the report in place (hidden on screen, replaced on the
  // next export). The browser's window.print() does fire it, so remove there.
  if (isTauri()) {
    await printDocument()
    return
  }
  const cleanup = () => {
    container.remove()
    window.removeEventListener('afterprint', cleanup)
  }
  window.addEventListener('afterprint', cleanup)
  await printDocument()
}
