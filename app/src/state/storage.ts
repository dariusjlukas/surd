// IndexedDB persistence for the notebook store, via idb-keyval. localStorage
// tops out around 5 MB — plot results carry 600 samples each, so a few
// plot-heavy notebooks blow through it. IDB raises the ceiling to "disk".
//
// Migration chain, run lazily on the first getItem that finds no v3 record:
//   v1 (localStorage, single notebook)  → v2 shape
//   v2 (localStorage, notebook list)    → v3: cells gain kind:'math', the
//      stored transcript is dropped (v3 derives it from cells — see store.ts)
// Legacy keys are removed after a successful write so the migration is
// one-shot.

import { del, get, set } from 'idb-keyval'
import type { StateStorage } from 'zustand/middleware'
import type { Cell, Notebook } from './store'

export const STORAGE_KEY = 'exact.notebooks.v3'
const V2_KEY = 'exact.notebooks.v2'
const V1_KEY = 'exact.notebook.v1'

export const idbStorage: StateStorage = {
  async getItem(name) {
    const stored = await get<string>(name)
    if (stored !== undefined) return stored
    const migrated = migrateFromLocalStorage()
    if (migrated !== null) {
      await set(name, migrated)
      localStorage.removeItem(V2_KEY)
      localStorage.removeItem(V1_KEY)
    }
    return migrated
  },
  async setItem(name, value) {
    await set(name, value)
  },
  async removeItem(name) {
    await del(name)
  },
}

type StoredCell = Omit<Cell, 'kind'> & { kind?: Cell['kind'] }
interface V2Notebook extends Omit<Notebook, 'cells'> {
  cells: StoredCell[]
  transcript?: string[]
}

function migrateFromLocalStorage(): string | null {
  try {
    const v2 = localStorage.getItem(V2_KEY)
    if (v2 !== null) {
      const { state } = JSON.parse(v2) as {
        state: {
          notebooks: V2Notebook[]
          activeId: string
          showWorkspace?: boolean
          showSidebar?: boolean
        }
      }
      return JSON.stringify({
        state: { ...state, notebooks: state.notebooks.map(upgradeNotebook) },
        version: 0,
      })
    }
    const v1 = localStorage.getItem(V1_KEY)
    if (v1 !== null) {
      const { state } = JSON.parse(v1) as {
        state: { cells?: StoredCell[]; transcript?: string[] }
      }
      const now = Date.now()
      const nb = upgradeNotebook({
        id: crypto.randomUUID(),
        name: 'Notebook 1',
        cells: state.cells ?? [],
        createdAt: now,
        updatedAt: now,
      })
      return JSON.stringify({
        state: { notebooks: [nb], activeId: nb.id },
        version: 0,
      })
    }
  } catch {
    // unreadable legacy payload — start fresh rather than refuse to boot
  }
  return null
}

function upgradeNotebook(nb: V2Notebook): Notebook {
  const rest = { ...nb }
  delete rest.transcript
  return {
    ...rest,
    cells: nb.cells.map((c) => ({ kind: 'math' as const, ...c })),
  }
}
