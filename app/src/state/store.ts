// Session state. Owns the EngineClient and all policy around it.
//
// Document model: a notebook is an ordered list of cells. Math cells carry
// engine results; markdown cells are narrative and never touch the engine.
// The transcript — the engine's workspace state — is DERIVED: the sources of
// successfully evaluated math cells, in order. The engine is deterministic,
// so replaying that derivation IS the workspace.
//
// Editing model: changing cell N (edit / delete / re-run) restarts the worker
// with the transcript of cells before N, then re-evaluates N and every math
// cell after it, in order. No dependency graph — replay is the consistency
// model. Cancel = EngineClient.restart(transcript): the worker dies, state
// replays.
//
// Persistence: notebooks + UI prefs go to IndexedDB (see storage.ts), so a
// reload paints instantly while the engine replays in the background.

import { create } from 'zustand'
import { createJSONStorage, persist } from 'zustand/middleware'
import { EngineCancelled, EngineClient } from '../engine/client'
import { initLexer } from '../engine/lexer'
import type {
  EvalResult,
  ReplayEntry,
  SamplePoint,
  WorkspaceEntry,
} from '../engine/types'
import { idbStorage, STORAGE_KEY } from './storage'

export type EngineStatus = 'booting' | 'restoring' | 'ready' | 'busy' | 'failed'

/** `data` cells are raw-data imports: they carry the imported file's text and
 * the workspace name it binds; `src` is only a display label. */
export type CellKind = 'math' | 'markdown' | 'data'

export interface Cell {
  id: string
  kind: CellKind
  src: string
  status: 'pending' | 'done' | 'cancelled'
  result?: EvalResult
  /** data cells: the workspace variable the import binds. */
  dataName?: string
  /** data cells: the imported file's raw text (replayed on engine restart). */
  dataPayload?: string
}

export interface Notebook {
  id: string
  name: string
  cells: Cell[]
  createdAt: number
  updatedAt: number
}

/** The engine state implied by a cell list: every successfully evaluated
 * math statement and every successful data import, in document order. */
export function transcriptOf(cells: Cell[]): ReplayEntry[] {
  return cells
    .filter((c) => c.status === 'done' && c.result?.ok)
    .flatMap((c): ReplayEntry[] => {
      if (c.kind === 'math') return [{ type: 'eval', src: c.src }]
      if (c.kind === 'data' && c.dataName && c.dataPayload) {
        return [{ type: 'import', name: c.dataName, payload: c.dataPayload }]
      }
      return []
    })
}

/** Reserved words a workspace variable may not shadow. Mirrors the engine's
 * lexer (plus `struct`, the constructor data imports rely on). */
const RESERVED = new Set([
  'if', 'then', 'else', 'end', 'while', 'do', 'function',
  'and', 'or', 'not', 'true', 'false', 'struct',
])

/** A valid, unused workspace name derived from an imported file's name. */
export function importVarName(fileName: string, taken: Iterable<string>): string {
  let base = fileName
    .replace(/\.[^.]*$/, '')
    .trim()
    .replace(/[^\p{L}\p{N}_]/gu, '_')
  if (!base) base = 'data'
  if (/^[0-9]/.test(base)) base = '_' + base
  if (RESERVED.has(base)) base += '_'
  const used = new Set(taken)
  let name = base
  for (let i = 2; used.has(name); i++) name = `${base}_${i}`
  return name
}

interface NotebookState {
  engineStatus: EngineStatus
  notebooks: Notebook[]
  activeId: string
  workspace: WorkspaceEntry[]
  showWorkspace: boolean
  showSidebar: boolean
  /** Settings view replaces the notebook area. Not persisted. */
  showSettings: boolean
  toggleWorkspace(): void
  toggleSidebar(): void
  toggleSettings(): void

  boot(): Promise<void>
  submit(src: string): Promise<void>
  /** Import a raw data file (exact-data/JSON/CSV) as a new data cell, bound
   * to a fresh workspace name derived from the file name. */
  importData(fileName: string, text: string): Promise<void>
  /** Serialize the named workspace variables into one exact-data file. */
  exportData(names: string[]): Promise<string>
  /** Re-evaluate a cell and everything below it (the cell's old effect on
   * the workspace must be undone, which means replaying the prefix). */
  rerun(cellId: string): Promise<void>
  /** Change a cell's source. Math cells recompute from that point down. */
  updateCell(cellId: string, src: string): Promise<void>
  deleteCell(cellId: string): Promise<void>
  /** Insert an empty cell after `afterId` (or append when null). Returns the
   * new cell's id; an empty cell opens in edit mode in the UI. */
  insertCell(afterId: string | null, kind: CellKind): string
  cancel(): void
  clearNotebook(): void
  resample(
    exprText: string,
    varName: string,
    a: number,
    b: number,
  ): Promise<SamplePoint[]>

  createNotebook(): void
  selectNotebook(id: string): void
  renameNotebook(id: string, name: string): void
  deleteNotebook(id: string): void
  /** Add an imported notebook and switch to it. */
  addNotebook(name: string, cells: Cell[]): void
}

function newNotebook(name: string): Notebook {
  const now = Date.now()
  return { id: crypto.randomUUID(), name, cells: [], createdAt: now, updatedAt: now }
}

/** A name like "Notebook 3" that no existing notebook already uses. */
export function untitledName(notebooks: Notebook[]): string {
  const taken = new Set(notebooks.map((n) => n.name))
  for (let i = 1; ; i++) {
    const name = `Notebook ${i}`
    if (!taken.has(name)) return name
  }
}

const client = new EngineClient()

const first = newNotebook('Notebook 1')

export const useNotebook = create<NotebookState>()(
  persist(
    (set, get) => {
      const refreshWorkspace = () => {
        client
          .workspace()
          .then((workspace) => set({ workspace }))
          .catch(() => {
            // engine restarted underneath us — the next refresh will land
          })
      }

      /** Immutably update one notebook (bumping updatedAt). */
      const patch = (id: string, fn: (n: Notebook) => Partial<Notebook>) =>
        set((s) => ({
          notebooks: s.notebooks.map((n) =>
            n.id === id ? { ...n, ...fn(n), updatedAt: Date.now() } : n,
          ),
        }))

      const active = () => {
        const s = get()
        return s.notebooks.find((n) => n.id === s.activeId) ?? s.notebooks[0]
      }

      /** Point the engine at `transcript`, with status bookkeeping. */
      const restartEngine = (transcript: ReplayEntry[]) => {
        set({ engineStatus: 'restoring', workspace: [] })
        client
          .restart(transcript)
          .then(() => {
            set({ engineStatus: 'ready' })
            refreshWorkspace()
          })
          .catch(() => set({ engineStatus: 'failed' }))
      }

      /** Mark any in-flight cells cancelled (used before engine restarts). */
      const cancelPending = () =>
        set((s) => ({
          notebooks: s.notebooks.map((n) => ({
            ...n,
            cells: n.cells.map((c) =>
              c.status === 'pending' ? { ...c, status: 'cancelled' as const } : c,
            ),
          })),
        }))

      const patchCell = (notebookId: string, cellId: string, p: Partial<Cell>) =>
        patch(notebookId, (n) => ({
          cells: n.cells.map((c) => (c.id === cellId ? { ...c, ...p } : c)),
        }))

      /** Evaluate `src` and write the outcome into an existing (pending)
       * cell. Used by submit; recomputeFrom inlines the same shape. */
      const evalIntoCell = async (notebookId: string, cellId: string, src: string) => {
        set({ engineStatus: 'busy' })
        try {
          const result = await client.eval(src)
          set({ engineStatus: 'ready' })
          patchCell(notebookId, cellId, { status: 'done', result })
          if (result.ok) refreshWorkspace()
        } catch (e) {
          if (!(e instanceof EngineCancelled)) throw e
          // cancel()/selectNotebook() already marked the cell and is
          // restarting the engine.
        }
      }

      /** The editing primitive: replay the workspace up to `index`, then
       * re-evaluate every math cell from `index` on, in document order. */
      const recomputeFrom = async (notebookId: string, index: number) => {
        const nb = get().notebooks.find((n) => n.id === notebookId)
        if (!nb) return
        const prefix = transcriptOf(nb.cells.slice(0, index))
        const tailIds = nb.cells
          .slice(index)
          .filter((c) => c.kind === 'math' || c.kind === 'data')
          .map((c) => c.id)
        patch(notebookId, (n) => ({
          cells: n.cells.map((c) =>
            tailIds.includes(c.id)
              ? { ...c, status: 'pending' as const, result: undefined }
              : c,
          ),
        }))
        set({ engineStatus: 'restoring', workspace: [] })
        try {
          await client.restart(prefix)
        } catch (e) {
          if (e instanceof EngineCancelled) return // superseded by another restart
          set({ engineStatus: 'failed' })
          return
        }
        set({ engineStatus: 'busy' })
        for (const cellId of tailIds) {
          const cell = get()
            .notebooks.find((n) => n.id === notebookId)
            ?.cells.find((c) => c.id === cellId)
          if (!cell) continue // deleted while we were evaluating earlier cells
          try {
            const result =
              cell.kind === 'data'
                ? await client.importData(cell.dataName ?? '', cell.dataPayload ?? '')
                : await client.eval(cell.src)
            patchCell(notebookId, cellId, { status: 'done', result })
          } catch (e) {
            if (!(e instanceof EngineCancelled)) throw e
            return // cancel()/selectNotebook() took over engine + statuses
          }
        }
        set({ engineStatus: 'ready' })
        refreshWorkspace()
      }

      return {
        engineStatus: 'booting',
        notebooks: [first],
        activeId: first.id,
        workspace: [],
        showWorkspace: true,
        showSidebar: true,
        showSettings: false,
        toggleWorkspace: () => set((s) => ({ showWorkspace: !s.showWorkspace })),
        toggleSidebar: () => set((s) => ({ showSidebar: !s.showSidebar })),
        toggleSettings: () => set((s) => ({ showSettings: !s.showSettings })),

        async boot() {
          const transcript = transcriptOf(active().cells)
          set({ engineStatus: transcript.length ? 'restoring' : 'booting' })
          try {
            await Promise.all([initLexer(), client.restart(transcript)])
            set({ engineStatus: 'ready' })
            refreshWorkspace()
          } catch (e) {
            console.error(e)
            set({ engineStatus: 'failed' })
          }
        },

        async submit(src: string) {
          if (get().engineStatus !== 'ready') return
          const notebookId = get().activeId
          const cell: Cell = {
            id: crypto.randomUUID(),
            kind: 'math',
            src,
            status: 'pending',
          }
          patch(notebookId, (n) => ({ cells: [...n.cells, cell] }))
          await evalIntoCell(notebookId, cell.id, src)
        },

        async importData(fileName: string, text: string) {
          if (get().engineStatus !== 'ready') return
          const notebookId = get().activeId
          const name = importVarName(
            fileName,
            get().workspace.map((w) => w.name),
          )
          const cell: Cell = {
            id: crypto.randomUUID(),
            kind: 'data',
            src: `import "${fileName}" as ${name}`,
            status: 'pending',
            dataName: name,
            dataPayload: text,
          }
          patch(notebookId, (n) => ({ cells: [...n.cells, cell] }))
          set({ engineStatus: 'busy' })
          try {
            const result = await client.importData(name, text)
            set({ engineStatus: 'ready' })
            patchCell(notebookId, cell.id, { status: 'done', result })
            if (result.ok) refreshWorkspace()
          } catch (e) {
            if (!(e instanceof EngineCancelled)) throw e
          }
        },

        exportData(names: string[]) {
          return client.exportData(names)
        },

        async rerun(cellId: string) {
          if (get().engineStatus !== 'ready') return
          const nb = active()
          const index = nb.cells.findIndex((c) => c.id === cellId)
          if (index < 0 || nb.cells[index].kind === 'markdown') return
          await recomputeFrom(nb.id, index)
        },

        async updateCell(cellId: string, src: string) {
          const nb = active()
          const index = nb.cells.findIndex((c) => c.id === cellId)
          if (index < 0) return
          if (nb.cells[index].kind === 'data') return // payload isn't editable
          if (nb.cells[index].kind === 'markdown') {
            patchCell(nb.id, cellId, { src })
            return
          }
          if (get().engineStatus !== 'ready') return
          patchCell(nb.id, cellId, { src })
          await recomputeFrom(nb.id, index)
        },

        async deleteCell(cellId: string) {
          const nb = active()
          const index = nb.cells.findIndex((c) => c.id === cellId)
          if (index < 0) return
          const cell = nb.cells[index]
          const affectsWorkspace =
            (cell.kind === 'math' || cell.kind === 'data') &&
            cell.status === 'done' &&
            cell.result?.ok
          if (affectsWorkspace && get().engineStatus !== 'ready') return
          patch(nb.id, (n) => ({ cells: n.cells.filter((c) => c.id !== cellId) }))
          // After removal, `index` is the first cell that saw the deleted
          // binding — recompute from there to undo its workspace effect.
          if (affectsWorkspace) await recomputeFrom(nb.id, index)
        },

        insertCell(afterId, kind) {
          const nb = active()
          const cell: Cell = {
            id: crypto.randomUUID(),
            kind,
            src: '',
            status: 'done',
          }
          patch(nb.id, (n) => {
            const at = afterId
              ? n.cells.findIndex((c) => c.id === afterId) + 1
              : n.cells.length
            return { cells: [...n.cells.slice(0, at), cell, ...n.cells.slice(at)] }
          })
          return cell.id
        },

        cancel() {
          if (get().engineStatus !== 'busy') return
          cancelPending()
          restartEngine(transcriptOf(active().cells))
        },

        clearNotebook() {
          patch(get().activeId, () => ({ cells: [] }))
          restartEngine([])
        },

        resample(exprText, varName, a, b) {
          return client.resample(exprText, varName, a, b)
        },

        createNotebook() {
          const nb = newNotebook(untitledName(get().notebooks))
          cancelPending()
          set((s) => ({ notebooks: [...s.notebooks, nb], activeId: nb.id }))
          restartEngine([])
        },

        selectNotebook(id) {
          const s = get()
          if (id === s.activeId) return
          const target = s.notebooks.find((n) => n.id === id)
          if (!target) return
          cancelPending()
          set({ activeId: id })
          restartEngine(transcriptOf(target.cells))
        },

        renameNotebook(id, name) {
          const trimmed = name.trim()
          if (!trimmed) return
          patch(id, () => ({ name: trimmed }))
        },

        deleteNotebook(id) {
          const s = get()
          const remaining = s.notebooks.filter((n) => n.id !== id)
          if (id !== s.activeId) {
            set({ notebooks: remaining })
            return
          }
          cancelPending()
          if (remaining.length === 0) {
            const nb = newNotebook('Notebook 1')
            set({ notebooks: [nb], activeId: nb.id })
            restartEngine([])
          } else {
            // Activate the nearest surviving neighbor.
            const idx = s.notebooks.findIndex((n) => n.id === id)
            const next = remaining[Math.min(idx, remaining.length - 1)]
            set({ notebooks: remaining, activeId: next.id })
            restartEngine(transcriptOf(next.cells))
          }
        },

        addNotebook(name, cells) {
          const nb = newNotebook(name)
          nb.cells = cells
          cancelPending()
          set((s) => ({ notebooks: [...s.notebooks, nb], activeId: nb.id }))
          restartEngine(transcriptOf(cells))
        },
      }
    },
    {
      name: STORAGE_KEY,
      storage: createJSONStorage(() => idbStorage),
      partialize: (s) => ({
        activeId: s.activeId,
        showWorkspace: s.showWorkspace,
        showSidebar: s.showSidebar,
        // Pending cells can't be rehydrated meaningfully; persist them as
        // cancelled so a mid-eval reload reads honestly.
        notebooks: s.notebooks.map((n) => ({
          ...n,
          cells: n.cells.map((c) =>
            c.status === 'pending' ? { ...c, status: 'cancelled' as const } : c,
          ),
        })),
      }),
    },
  ),
)

/** The active notebook (always defined — the store never holds zero). */
export function useActiveNotebook(): Notebook {
  return useNotebook(
    (s) => s.notebooks.find((n) => n.id === s.activeId) ?? s.notebooks[0],
  )
}

/** Input history for ↑/↓ recall: successful inputs plus the failed ones the
 * user probably wants to fix — i.e. every submitted source in this notebook. */
export function useInputHistory(): string[] {
  return useActiveNotebook()
    .cells.filter((c) => c.kind === 'math')
    .map((c) => c.src)
}
