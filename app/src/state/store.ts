// Session state. Owns the EngineClient and all policy around it.
//
// Persistence model: two things are saved to localStorage —
//   - `transcript`: every successful input, in order. The engine is
//     deterministic, so replaying the transcript IS restoring the workspace.
//   - `cells`: the rendered notebook (inputs + cached results), so a reload
//     paints instantly while the engine replays in the background.
// Cancel = EngineClient.restart(transcript): the worker dies, state replays.

import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { EngineCancelled, EngineClient } from '../engine/client'
import { initLexer } from '../engine/lexer'
import type { EvalResult, SamplePoint, WorkspaceEntry } from '../engine/types'

export type EngineStatus = 'booting' | 'restoring' | 'ready' | 'busy' | 'failed'

export interface Cell {
  id: string
  src: string
  status: 'pending' | 'done' | 'cancelled'
  result?: EvalResult
}

interface NotebookState {
  engineStatus: EngineStatus
  cells: Cell[]
  transcript: string[]
  workspace: WorkspaceEntry[]
  /** UI: variables panel visibility (not persisted). */
  showWorkspace: boolean
  toggleWorkspace(): void

  boot(): Promise<void>
  submit(src: string): Promise<void>
  cancel(): void
  clearWorkspace(): void
  resample(
    exprText: string,
    varName: string,
    a: number,
    b: number,
  ): Promise<SamplePoint[]>
}

/** Cap on persisted cells — localStorage is ~5 MB and plot results carry 600
 * samples each. The transcript (engine state) is never trimmed. */
const MAX_PERSISTED_CELLS = 200

const client = new EngineClient()

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

      return {
        engineStatus: 'booting',
        cells: [],
        transcript: [],
        workspace: [],
        showWorkspace: true,
        toggleWorkspace: () => set((s) => ({ showWorkspace: !s.showWorkspace })),

        async boot() {
          set({ engineStatus: get().transcript.length ? 'restoring' : 'booting' })
          try {
            await Promise.all([initLexer(), client.restart(get().transcript)])
            set({ engineStatus: 'ready' })
            refreshWorkspace()
          } catch (e) {
            console.error(e)
            set({ engineStatus: 'failed' })
          }
        },

        async submit(src: string) {
          if (get().engineStatus !== 'ready') return
          const cell: Cell = { id: crypto.randomUUID(), src, status: 'pending' }
          set((s) => ({ cells: [...s.cells, cell], engineStatus: 'busy' }))
          try {
            const result = await client.eval(src)
            set((s) => ({
              engineStatus: 'ready',
              cells: s.cells.map((c) =>
                c.id === cell.id ? { ...c, status: 'done' as const, result } : c,
              ),
              // Only successful inputs become workspace state.
              transcript: result.ok ? [...s.transcript, src] : s.transcript,
            }))
            if (result.ok) refreshWorkspace()
          } catch (e) {
            if (!(e instanceof EngineCancelled)) throw e
            // cancel() already marked the cell and is restarting the engine.
          }
        },

        cancel() {
          const { engineStatus, transcript } = get()
          if (engineStatus !== 'busy') return
          set((s) => ({
            engineStatus: 'restoring',
            cells: s.cells.map((c) =>
              c.status === 'pending' ? { ...c, status: 'cancelled' as const } : c,
            ),
          }))
          client
            .restart(transcript)
            .then(() => {
              set({ engineStatus: 'ready' })
              refreshWorkspace()
            })
            .catch(() => set({ engineStatus: 'failed' }))
        },

        clearWorkspace() {
          set({ cells: [], transcript: [], workspace: [], engineStatus: 'restoring' })
          client
            .restart([])
            .then(() => set({ engineStatus: 'ready' }))
            .catch(() => set({ engineStatus: 'failed' }))
        },

        resample(exprText, varName, a, b) {
          return client.resample(exprText, varName, a, b)
        },
      }
    },
    {
      name: 'exact.notebook.v1',
      partialize: (s) => ({
        transcript: s.transcript,
        // Pending cells can't be rehydrated meaningfully; persist them as
        // cancelled so a mid-eval reload reads honestly.
        cells: s.cells.slice(-MAX_PERSISTED_CELLS).map((c) =>
          c.status === 'pending' ? { ...c, status: 'cancelled' as const } : c,
        ),
      }),
    },
  ),
)

/** Input history for ↑/↓ recall: successful inputs plus the failed ones the
 * user probably wants to fix — i.e. every submitted source, deduped tail. */
export function useInputHistory(): string[] {
  return useNotebook((s) => s.cells).map((c) => c.src)
}
