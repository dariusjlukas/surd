// One-slot undo: destructive notebook operations (delete cell, delete
// notebook, clear notebook) apply immediately and offer a transient toast to
// reverse them — better than a blocking confirm() on both safety and
// friction. Latest offer wins; each offer auto-expires. The undo action must
// capture everything it needs to restore: replay is the consistency model,
// so re-inserting the cells and recomputing IS the undo.

import { create } from 'zustand'

const TOAST_MS = 10_000

interface UndoState {
  toast: { id: number; message: string; undo: () => void } | null
  offer(message: string, undo: () => void): void
  /** Run the pending undo (if any) and dismiss the toast. */
  run(): void
  dismiss(): void
}

let nextId = 1
let timer: ReturnType<typeof setTimeout> | undefined

export const useUndo = create<UndoState>()((set, get) => ({
  toast: null,
  offer(message, undo) {
    clearTimeout(timer)
    const id = nextId++
    timer = setTimeout(() => {
      // Only expire our own toast — a newer offer owns the slot now.
      if (get().toast?.id === id) set({ toast: null })
    }, TOAST_MS)
    set({ toast: { id, message, undo } })
  },
  run() {
    const t = get().toast
    if (!t) return
    set({ toast: null })
    t.undo()
  },
  dismiss() {
    clearTimeout(timer)
    set({ toast: null })
  },
}))

/** Store-side entry point (the notebook store offers undos from its
 * destructive actions without importing React anything). */
export const offerUndo = (message: string, undo: () => void) =>
  useUndo.getState().offer(message, undo)
