// Stale-dependency analysis. A cell's shown result was computed from its
// committed source; an in-progress edit (a "draft") makes that result, and the
// results of every later cell that transitively reads what the edit changes,
// STALE until a re-run. The notebook has no dependency graph — replay is its
// consistency model — so we derive dependencies here from each cell's def/use
// symbols (extracted by the engine's real parser, see wasm cell_symbols).
//
// Drafts live in their own un-persisted store so per-keystroke edits never
// touch the notebook store (or IndexedDB); only NotebookView subscribes, and
// it re-derives the stale set once per change.

import { create } from 'zustand'
import { cell_symbols } from '../engine/lexer'
import type { Cell } from './store'

interface Symbols {
  /** Workspace names the cell binds (unconditional top-level defs). */
  defs: string[]
  /** Free workspace names the cell reads. */
  uses: string[]
}

const EMPTY: Symbols = { defs: [], uses: [] }

// Symbol extraction parses, so memoize by source text. Drafts churn one new
// string per keystroke; bound the cache rather than let it grow unbounded.
const cache = new Map<string, Symbols>()

function symbolsOfSource(src: string): Symbols {
  const hit = cache.get(src)
  if (hit) return hit
  let sym: Symbols
  try {
    // Throws only if the lexer wasm isn't initialized yet (early boot); an
    // unparseable draft comes back as empty sets, not an error.
    sym = JSON.parse(cell_symbols(src)) as Symbols
  } catch {
    sym = EMPTY
  }
  if (cache.size > 1000) cache.clear()
  cache.set(src, sym)
  return sym
}

/** Defs/uses of a cell's committed source. Data cells bind their import name
 * and read nothing; markdown cells touch no workspace symbols. */
function symbolsOfCell(cell: Cell): Symbols {
  if (cell.kind === 'markdown') return EMPTY
  if (cell.kind === 'data')
    return { defs: cell.dataName ? [cell.dataName] : [], uses: [] }
  return symbolsOfSource(cell.src)
}

/** The ids of cells whose shown result is stale given the current drafts.
 *
 * One forward pass in document order carries a `dirty` set of workspace names
 * whose value is out of date. A cell is stale if it has its own pending edit,
 * or it reads a dirty name; either way its own bindings then join `dirty` so
 * the staleness propagates. A clean, up-to-date cell's bindings "heal" — they
 * leave `dirty`, since its unconditional defs now hold the current value. */
export function computeStaleCells(
  cells: Cell[],
  drafts: Record<string, string>,
): Set<string> {
  const stale = new Set<string>()
  const dirty = new Set<string>()
  for (const cell of cells) {
    if (cell.kind === 'markdown') continue
    const { defs, uses } = symbolsOfCell(cell)
    const draft = drafts[cell.id]
    const edited = draft !== undefined && draft !== cell.src
    const readsDirty = uses.some((u) => dirty.has(u))
    if (edited || readsDirty) stale.add(cell.id)
    if (edited) {
      // Both the old bindings and the draft's new ones will change on re-run.
      for (const d of defs) dirty.add(d)
      for (const d of symbolsOfSource(draft).defs) dirty.add(d)
    } else if (readsDirty) {
      for (const d of defs) dirty.add(d)
    } else {
      for (const d of defs) dirty.delete(d)
    }
  }
  return stale
}

interface DraftState {
  /** cellId → the editor's current text while a cell is being edited. */
  drafts: Record<string, string>
  setDraft(id: string, doc: string): void
  clearDraft(id: string): void
}

export const useDrafts = create<DraftState>((set) => ({
  drafts: {},
  setDraft: (id, doc) =>
    set((s) =>
      s.drafts[id] === doc ? s : { drafts: { ...s.drafts, [id]: doc } },
    ),
  clearDraft: (id) =>
    set((s) => {
      if (!(id in s.drafts)) return s
      const drafts = { ...s.drafts }
      delete drafts[id]
      return { drafts }
    }),
}))
