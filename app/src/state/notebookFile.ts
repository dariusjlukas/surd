// Notebook ⇄ .json file. The file carries the cells (the document); the
// engine workspace is derived from them by deterministic replay, exactly as
// in the store. A `transcript` field is written for readability and for
// other tools, but on import the cells are authoritative.
//
// Version history: v1 files predate markdown cells (no `kind`) and carried a
// stored transcript; v2 adds `kind`. Both import.

import type { Cell, Notebook } from './store'
import { transcriptOf } from './store'
import type { EvalResult } from '../engine/types'

const FORMAT = 'exact-notebook'
const VERSION = 2

interface FileCell {
  kind?: Cell['kind']
  src: string
  status: Cell['status']
  result?: EvalResult
}

interface NotebookFile {
  format: typeof FORMAT
  version: number
  name: string
  transcript: string[]
  cells: FileCell[]
}

export function serializeNotebook(nb: Notebook): string {
  const file: NotebookFile = {
    format: FORMAT,
    version: VERSION,
    name: nb.name,
    transcript: transcriptOf(nb.cells),
    cells: nb.cells.map(({ kind, src, status, result }) => ({
      kind,
      src,
      // A pending cell has no result to carry; export it as cancelled.
      status: status === 'pending' ? 'cancelled' : status,
      result,
    })),
  }
  return JSON.stringify(file, null, 2)
}

export function downloadNotebook(nb: Notebook) {
  const blob = new Blob([serializeNotebook(nb)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${nb.name.replace(/[/\\:*?"<>|]/g, '_')}.json`
  a.click()
  URL.revokeObjectURL(url)
}

/** Parse an exported notebook. Throws with a user-facing message on any
 * shape problem — the caller surfaces it verbatim. */
export function parseNotebookFile(text: string): { name: string; cells: Cell[] } {
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch {
    throw new Error('not a JSON file')
  }
  if (typeof data !== 'object' || data === null) throw new Error('not a notebook file')
  const f = data as Partial<NotebookFile>
  if (f.format !== FORMAT) throw new Error('not an exact notebook file')
  if (typeof f.version !== 'number' || f.version > VERSION) {
    throw new Error(`unsupported notebook version ${String(f.version)}`)
  }
  const rawCells = Array.isArray(f.cells) ? f.cells : []
  const cells: Cell[] = rawCells
    .filter((c) => typeof c?.src === 'string')
    .map((c) => ({
      id: crypto.randomUUID(),
      kind: c.kind === 'markdown' ? 'markdown' : 'math',
      src: c.src,
      status: c.status === 'done' ? 'done' : 'cancelled',
      result: c.result,
    }))
  return {
    name: typeof f.name === 'string' && f.name.trim() ? f.name.trim() : 'Imported',
    cells,
  }
}
