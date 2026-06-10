import { useNotebook } from '../state/store'

const LABEL: Record<string, string> = {
  booting: 'loading engine…',
  restoring: 'restoring workspace…',
  ready: 'ready',
  busy: 'evaluating…',
  failed: 'engine failed to load — reload the page',
}

export function StatusBar() {
  const engineStatus = useNotebook((s) => s.engineStatus)
  const cancel = useNotebook((s) => s.cancel)
  const clearWorkspace = useNotebook((s) => s.clearWorkspace)
  const showWorkspace = useNotebook((s) => s.showWorkspace)
  const toggleWorkspace = useNotebook((s) => s.toggleWorkspace)

  return (
    <header className="flex items-baseline gap-3 border-b border-slate-800 px-4 py-2.5 sm:px-6">
      <h1 className="font-mono text-base font-semibold text-sky-400">exact</h1>
      <p className="hidden flex-1 truncate text-xs text-slate-500 sm:block">
        exact by default — <code>:=</code> assigns, <code>N(x)</code> goes numeric,{' '}
        <code>plot(f, x, a, b)</code> draws
      </p>
      <span className="ml-auto text-xs text-slate-500">{LABEL[engineStatus]}</span>
      {engineStatus === 'busy' && (
        <button
          onClick={cancel}
          className="rounded bg-rose-500/90 px-2.5 py-0.5 text-xs font-medium text-slate-950 hover:bg-rose-400"
        >
          cancel
        </button>
      )}
      <button
        onClick={toggleWorkspace}
        className={`rounded border px-2.5 py-0.5 text-xs ${
          showWorkspace
            ? 'border-sky-700 text-sky-300'
            : 'border-slate-700 text-slate-400 hover:border-slate-500 hover:text-slate-200'
        }`}
      >
        vars
      </button>
      <button
        onClick={() => {
          if (window.confirm('Clear the saved workspace and notebook?')) clearWorkspace()
        }}
        className="rounded border border-slate-700 px-2.5 py-0.5 text-xs text-slate-400 hover:border-slate-500 hover:text-slate-200"
      >
        clear
      </button>
    </header>
  )
}
