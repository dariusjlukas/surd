// Transient bottom toast for destructive operations: message + Undo.
// Cmd/Ctrl+Z also triggers the pending undo — but only when focus is outside
// a text editor, which owns its local undo stack.

import { useEffect } from 'react'
import { useUndo } from '../state/undo'

export function UndoToast() {
  const toast = useUndo((s) => s.toast)
  const run = useUndo((s) => s.run)
  const dismiss = useUndo((s) => s.dismiss)

  useEffect(() => {
    if (!toast) return
    const onKey = (e: KeyboardEvent) => {
      if (!(e.key === 'z' && (e.metaKey || e.ctrlKey) && !e.shiftKey)) return
      const t = e.target as HTMLElement | null
      if (t?.closest('.cm-editor, input, textarea, [contenteditable]')) return
      e.preventDefault()
      run()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toast, run])

  if (!toast) return null
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-20 z-50 flex justify-center px-4">
      <div className="pointer-events-auto flex items-center gap-3 rounded-md border border-edge bg-raised px-3 py-2 text-sm shadow-lg">
        <span className="min-w-0 truncate text-muted">{toast.message}</span>
        <button
          onClick={run}
          className="shrink-0 font-medium text-accent hover:underline"
        >
          Undo
        </button>
        <button
          onClick={dismiss}
          title="dismiss"
          aria-label="dismiss"
          className="shrink-0 text-faint hover:text-ink"
        >
          ×
        </button>
      </div>
    </div>
  )
}
