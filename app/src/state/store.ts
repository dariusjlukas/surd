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
import {
  EngineCancelled,
  EngineClient,
  type Sampled3d,
  type SampledCurve,
} from '../engine/client'
import { initLexer } from '../engine/lexer'
import type {
  EvalResult,
  ImportFormat,
  RawExportFormat,
  ReplayEntry,
  WorkspaceEntry,
} from '../engine/types'
import { offerUndo } from './undo'
import { idbStorage, STORAGE_KEY } from './storage'

export type EngineStatus = 'booting' | 'restoring' | 'ready' | 'busy' | 'failed'

/** `data` cells are raw-data imports: they carry the imported file's text and
 * the workspace name it binds; `src` is only a display label. */
export type CellKind = 'math' | 'markdown' | 'data'

/** Where to drop a freshly inserted cell: next to a neighbor (`{after}` /
 * `{before}` carry that neighbor's id) or at either end of the notebook. */
export type InsertPos = { after: string } | { before: string } | 'start' | 'end'

export interface Cell {
  id: string
  kind: CellKind
  src: string
  status: 'pending' | 'done' | 'cancelled'
  result?: EvalResult
  /** Wall-clock evaluation time of the last run, ms. Absent for markdown
   * cells and results saved before this field existed. */
  ms?: number
  /** data cells: the workspace variable the import binds. */
  dataName?: string
  /** data cells: the imported file's raw text — or base64 bytes for binary
   * bulk formats (replayed on engine restart). */
  dataPayload?: string
  /** data cells: how the payload is parsed (absent = 'auto', the original
   * exact text-import path). */
  dataFormat?: ImportFormat
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
        return [
          {
            type: 'import',
            name: c.dataName,
            payload: c.dataPayload,
            format: c.dataFormat,
          },
        ]
      }
      return []
    })
}

/** Reserved words a workspace variable may not shadow. Mirrors the engine's
 * lexer (plus `struct`, the constructor data imports rely on). */
const RESERVED = new Set([
  'if',
  'then',
  'else',
  'end',
  'while',
  'do',
  'function',
  'and',
  'or',
  'not',
  'true',
  'false',
  'struct',
])

/** A valid, unused workspace name derived from an imported file's name. */
export function importVarName(
  fileName: string,
  taken: Iterable<string>,
): string {
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
  /** Which side panel is open as an overlay on phone-width screens. The
   * desktop `showSidebar`/`showWorkspace` prefs are pinned columns and don't
   * apply on mobile, so this is a separate, session-only (unpersisted) state
   * that starts closed — a phone gets a clean single column on first load. */
  mobileDrawer: 'sidebar' | 'workspace' | null
  toggleWorkspace(): void
  toggleSidebar(): void
  toggleSettings(): void
  toggleMobileDrawer(which: 'sidebar' | 'workspace'): void
  closeMobileDrawer(): void

  boot(): Promise<void>
  submit(src: string): Promise<void>
  /** Import a raw data file (surd-data/JSON/CSV) as a new data cell, bound
   * to a fresh workspace name derived from the file name. */
  importData(
    fileName: string,
    payload: string,
    format?: ImportFormat,
  ): Promise<void>
  /** Serialize the named workspace variables into one surd-data file. */
  exportData(names: string[]): Promise<string>
  /** Export one variable as raw little-endian binary; resolves to base64. */
  exportRaw(name: string, format: RawExportFormat): Promise<string>
  /** Re-evaluate a cell and everything below it (the cell's old effect on
   * the workspace must be undone, which means replaying the prefix). */
  rerun(cellId: string): Promise<void>
  /** Change a cell's source. Math cells recompute from that point down. */
  updateCell(cellId: string, src: string): Promise<void>
  /** Convert a cell between code (`math`) and formatted text (`markdown`) in
   * place — the source text is reinterpreted. The engine state replays from
   * that cell down (a former binding is undone; a new one is evaluated), so
   * this needs the engine ready. Data cells aren't convertible. */
  setCellKind(cellId: string, kind: 'math' | 'markdown'): Promise<void>
  deleteCell(cellId: string): Promise<void>
  /** Insert an empty cell at `pos` (next to a neighbor, or at an end). Returns
   * the new cell's id; an empty cell opens in edit mode in the UI. */
  insertCell(pos: InsertPos, kind: CellKind): string
  cancel(): void
  clearNotebook(): void
  resample(
    exprText: string,
    varName: string,
    a: number,
    b: number,
  ): Promise<SampledCurve>
  resampleSignal(
    sig: number,
    series: number,
    a: number,
    b: number,
  ): Promise<SampledCurve>
  resample3d(
    exprText: string,
    xvar: string,
    yvar: string,
    a: number,
    b: number,
    c: number,
    d: number,
  ): Promise<Sampled3d>

  createNotebook(): void
  selectNotebook(id: string): void
  renameNotebook(id: string, name: string): void
  deleteNotebook(id: string): void
  /** Add an imported notebook and switch to it. */
  addNotebook(name: string, cells: Cell[]): void
  /** Create a notebook from a built-in example (sources only) and evaluate
   * it live — every result on screen comes from the user's own engine. */
  openExample(
    name: string,
    cells: readonly { kind: 'math' | 'markdown'; src: string }[],
  ): Promise<void>
}

function newNotebook(name: string): Notebook {
  const now = Date.now()
  return {
    id: crypto.randomUUID(),
    name,
    cells: [],
    createdAt: now,
    updatedAt: now,
  }
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
              c.status === 'pending'
                ? { ...c, status: 'cancelled' as const }
                : c,
            ),
          })),
        }))

      const patchCell = (
        notebookId: string,
        cellId: string,
        p: Partial<Cell>,
      ) =>
        patch(notebookId, (n) => ({
          cells: n.cells.map((c) => (c.id === cellId ? { ...c, ...p } : c)),
        }))

      /** Evaluate `src` and write the outcome into an existing (pending)
       * cell. Used by submit; recomputeFrom inlines the same shape. */
      const evalIntoCell = async (
        notebookId: string,
        cellId: string,
        src: string,
      ) => {
        set({ engineStatus: 'busy' })
        try {
          const t0 = performance.now()
          const result = await client.eval(src)
          const ms = performance.now() - t0
          set({ engineStatus: 'ready' })
          patchCell(notebookId, cellId, { status: 'done', result, ms })
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
            const t0 = performance.now()
            const result =
              cell.kind === 'data'
                ? await client.importData(
                    cell.dataName ?? '',
                    cell.dataPayload ?? '',
                    cell.dataFormat,
                  )
                : await client.eval(cell.src)
            const ms = performance.now() - t0
            patchCell(notebookId, cellId, { status: 'done', result, ms })
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
        mobileDrawer: null,
        toggleWorkspace: () =>
          set((s) => ({ showWorkspace: !s.showWorkspace })),
        toggleSidebar: () => set((s) => ({ showSidebar: !s.showSidebar })),
        toggleSettings: () => set((s) => ({ showSettings: !s.showSettings })),
        toggleMobileDrawer: (which) =>
          set((s) => ({
            mobileDrawer: s.mobileDrawer === which ? null : which,
          })),
        closeMobileDrawer: () => set({ mobileDrawer: null }),

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

        async importData(
          fileName: string,
          payload: string,
          format?: ImportFormat,
        ) {
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
            dataPayload: payload,
            dataFormat: format,
          }
          patch(notebookId, (n) => ({ cells: [...n.cells, cell] }))
          set({ engineStatus: 'busy' })
          try {
            const t0 = performance.now()
            const result = await client.importData(name, payload, format)
            const ms = performance.now() - t0
            set({ engineStatus: 'ready' })
            patchCell(notebookId, cell.id, { status: 'done', result, ms })
            if (result.ok) refreshWorkspace()
          } catch (e) {
            if (!(e instanceof EngineCancelled)) throw e
          }
        },

        exportData(names: string[]) {
          return client.exportData(names)
        },

        exportRaw(name: string, format: RawExportFormat) {
          return client.exportRaw(name, format)
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

        async setCellKind(cellId, kind) {
          if (get().engineStatus !== 'ready') return
          const nb = active()
          const index = nb.cells.findIndex((c) => c.id === cellId)
          if (index < 0) return
          const cell = nb.cells[index]
          if (cell.kind === 'data' || cell.kind === kind) return
          // A math cell that bound a value (ran cleanly) needs that effect
          // undone; becoming math always needs evaluating. Either way, replay
          // from here. A never-bound math→markdown is a pure relabel.
          const undoesBinding =
            cell.kind === 'math' && cell.status === 'done' && !!cell.result?.ok
          const needsReplay = kind === 'math' || undoesBinding
          patchCell(nb.id, cellId, {
            kind,
            result: undefined,
            status: kind === 'math' ? 'pending' : 'done',
          })
          if (needsReplay) await recomputeFrom(nb.id, index)
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
          patch(nb.id, (n) => ({
            cells: n.cells.filter((c) => c.id !== cellId),
          }))
          // Offer to reverse it — but not for abandoned empty inserts, which
          // also arrive here (committing/blurring an empty editor deletes).
          if (cell.src.trim() !== '') {
            const label =
              cell.kind === 'markdown'
                ? 'text cell'
                : cell.kind === 'data'
                  ? `data import (${cell.dataName})`
                  : 'code cell'
            offerUndo(`Deleted ${label}`, () => {
              // Re-insert at the old spot, clamped — neighbors may be gone.
              patch(nb.id, (n) => {
                const at = Math.min(index, n.cells.length)
                return {
                  cells: [...n.cells.slice(0, at), cell, ...n.cells.slice(at)],
                }
              })
              // Replaying from the restored cell re-establishes its binding
              // (and re-runs everything below it, same as the delete did).
              if (affectsWorkspace) void recomputeFrom(nb.id, index)
            })
          }
          // After removal, `index` is the first cell that saw the deleted
          // binding — recompute from there to undo its workspace effect.
          if (affectsWorkspace) await recomputeFrom(nb.id, index)
        },

        insertCell(pos, kind) {
          const nb = active()
          const cell: Cell = {
            id: crypto.randomUUID(),
            kind,
            src: '',
            status: 'done',
          }
          patch(nb.id, (n) => {
            const at =
              pos === 'start'
                ? 0
                : pos === 'end'
                  ? n.cells.length
                  : 'after' in pos
                    ? n.cells.findIndex((c) => c.id === pos.after) + 1
                    : n.cells.findIndex((c) => c.id === pos.before)
            // A neighbor id we can't find (concurrent delete) → append.
            const idx = at < 0 ? n.cells.length : at
            return {
              cells: [...n.cells.slice(0, idx), cell, ...n.cells.slice(idx)],
            }
          })
          return cell.id
        },

        cancel() {
          if (get().engineStatus !== 'busy') return
          cancelPending()
          restartEngine(transcriptOf(active().cells))
        },

        clearNotebook() {
          const nb = active()
          if (nb.cells.length === 0) return
          const cells = nb.cells
          patch(nb.id, () => ({ cells: [] }))
          restartEngine([])
          offerUndo(`Cleared “${nb.name}”`, () => {
            patch(nb.id, () => ({ cells }))
            // Restoring is only meaningful if that notebook is still the one
            // on screen; if the user switched away, the patch alone is right.
            if (get().activeId === nb.id) {
              restartEngine(transcriptOf(cells))
            }
          })
        },

        resample(exprText, varName, a, b) {
          return client.resample(exprText, varName, a, b)
        },

        resampleSignal(sig, series, a, b) {
          return client.resampleSignal(sig, series, a, b)
        },

        resample3d(exprText, xvar, yvar, a, b, c, d) {
          return client.resample3d(exprText, xvar, yvar, a, b, c, d)
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
          const idx = s.notebooks.findIndex((n) => n.id === id)
          if (idx < 0) return
          const deleted = s.notebooks[idx]
          const remaining = s.notebooks.filter((n) => n.id !== id)
          // If this was the last notebook, a blank placeholder takes its
          // place; remember it so an undo can drop it again if untouched.
          let placeholderId: string | null = null
          if (id !== s.activeId) {
            set({ notebooks: remaining })
          } else {
            cancelPending()
            if (remaining.length === 0) {
              const nb = newNotebook('Notebook 1')
              placeholderId = nb.id
              set({ notebooks: [nb], activeId: nb.id })
              restartEngine([])
            } else {
              // Activate the nearest surviving neighbor.
              const next = remaining[Math.min(idx, remaining.length - 1)]
              set({ notebooks: remaining, activeId: next.id })
              restartEngine(transcriptOf(next.cells))
            }
          }
          offerUndo(`Deleted notebook “${deleted.name}”`, () => {
            const cur = get()
            if (cur.notebooks.some((n) => n.id === deleted.id)) return
            const notebooks = cur.notebooks.filter(
              // Drop the placeholder again, but only while it's still blank.
              (n) => !(n.id === placeholderId && n.cells.length === 0),
            )
            const at = Math.min(idx, notebooks.length)
            cancelPending()
            set({
              notebooks: [
                ...notebooks.slice(0, at),
                deleted,
                ...notebooks.slice(at),
              ],
              activeId: deleted.id,
            })
            restartEngine(transcriptOf(deleted.cells))
          })
        },

        addNotebook(name, cells) {
          const nb = newNotebook(name)
          nb.cells = cells
          cancelPending()
          set((s) => ({ notebooks: [...s.notebooks, nb], activeId: nb.id }))
          restartEngine(transcriptOf(cells))
        },

        async openExample(name, cells) {
          const s = get()
          if (s.engineStatus === 'booting' || s.engineStatus === 'failed')
            return
          const taken = new Set(s.notebooks.map((n) => n.name))
          let unique = name
          for (let i = 2; taken.has(unique); i++) unique = `${name} (${i})`
          const nb = newNotebook(unique)
          nb.cells = cells.map((c) => ({
            id: crypto.randomUUID(),
            kind: c.kind,
            src: c.src,
            status:
              c.kind === 'math' ? ('pending' as const) : ('done' as const),
          }))
          cancelPending()
          set((st) => ({ notebooks: [...st.notebooks, nb], activeId: nb.id }))
          // Replay from the top: restarts the engine clean and evaluates
          // every math cell in order, exactly like editing cell 0 would.
          await recomputeFrom(nb.id, 0)
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
