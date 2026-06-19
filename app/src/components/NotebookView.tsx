import { useEffect, useMemo, useRef } from 'react'
import { useSettings } from '../state/settings'
import { useActiveNotebook, useNotebook } from '../state/store'
import { computeStaleCells, useDrafts } from '../state/staleness'
import { CellView } from './CellView'
import { openContextMenu } from '../state/contextMenu'

export function NotebookView() {
  const notebook = useActiveNotebook()
  const insertCell = useNotebook((s) => s.insertCell)
  const clearNotebook = useNotebook((s) => s.clearNotebook)
  const confirmDelete = useSettings((s) => s.confirmDelete)
  const autoScroll = useSettings((s) => s.autoScroll)
  const endRef = useRef<HTMLDivElement>(null)
  const count = notebook.cells.length

  // An in-progress edit anywhere can stale this cell or any later one that
  // reads what it changes; recompute the set once and hand each cell a bool.
  const drafts = useDrafts((s) => s.drafts)
  const stale = useMemo(
    () => computeStaleCells(notebook.cells, drafts),
    [notebook.cells, drafts],
  )

  useEffect(() => {
    if (autoScroll) endRef.current?.scrollIntoView({ block: 'end' })
  }, [count, notebook.id, autoScroll])

  return (
    <div
      className="flex-1 space-y-4 overflow-y-auto px-4 py-4 sm:px-6"
      onContextMenu={(e) =>
        openContextMenu(e, [
          {
            label: 'Add text cell',
            onSelect: () => insertCell(null, 'markdown'),
          },
          'divider',
          {
            label: 'Clear notebook…',
            danger: true,
            disabled: count === 0,
            onSelect: () => {
              if (
                !confirmDelete ||
                window.confirm(
                  `Clear "${notebook.name}" — its cells and workspace?`,
                )
              ) {
                clearNotebook()
              }
            },
          },
        ])
      }
    >
      {count === 0 && <Welcome />}
      {notebook.cells.map((c) => (
        <CellView key={c.id} cell={c} stale={stale.has(c.id)} />
      ))}
      <div ref={endRef} />
    </div>
  )
}

function Welcome() {
  const submit = useNotebook((s) => s.submit)
  const ready = useNotebook((s) => s.engineStatus === 'ready')
  const examples = [
    '1/3 + 1/6',
    'sqrt(2)*sqrt(2)',
    'inv([1/2, 1/3; 1/4, 1/5])',
    'diff(sin(x)^2, x)',
    'N(pi, 50)',
    'plot(sin(x)/x, x, -15, 15)',
    'fact(n) := if n == 0 then 1 else n*fact(n-1) end',
  ]
  return (
    <div className="mx-auto mt-8 max-w-xl text-sm text-faint">
      <p className="mb-3">
        Exact by default: <code className="text-muted">1/3</code> stays a third,{' '}
        <code className="text-muted">sqrt(2)</code> stays a radical. Floats only
        via <code className="text-muted">N(x)</code>. Try one:
      </p>
      <ul className="space-y-1 font-mono">
        {examples.map((e) => (
          <li key={e}>
            <button
              onClick={() => void submit(e)}
              disabled={!ready}
              className="rounded-md px-1.5 py-0.5 text-left text-muted hover:bg-hover/80 hover:text-accent disabled:cursor-default disabled:hover:bg-transparent disabled:hover:text-muted"
            >
              {e}
            </button>
          </li>
        ))}
      </ul>
      <p className="mt-4 text-xs text-faint">
        <code>:=</code> assigns · <code>plot(f, x, a, b)</code> draws · ↑/↓
        recalls history · click a cell to edit it (re-running recomputes
        everything below) · <em>+ note</em> adds markdown · notebooks save
        automatically
      </p>
    </div>
  )
}
