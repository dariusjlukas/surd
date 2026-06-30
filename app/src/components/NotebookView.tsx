import { Fragment, useEffect, useMemo, useRef } from 'react'
import { useSettings } from '../state/settings'
import { useActiveNotebook, useNotebook, type InsertPos } from '../state/store'
import { computeStaleCells, useDrafts } from '../state/staleness'
import { CellView } from './CellView'
import { exportNotebookPdf } from './exportPdf'
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

  // Keep the right cell in view as the list changes. Entering a notebook (mount
  // or switch) and appending a cell (input-bar submit, import, or an "end"
  // insert) snap to the bottom so the latest row shows. A *middle* insert must
  // NOT jump to the bottom — that buries the just-created cell the user is about
  // to edit (see the +code/+text inserters); scroll only that cell into view.
  // Deletes and bulk changes (added.length ≠ 1) leave the scroll position be.
  const prevIds = useRef<string[] | null>(null)
  const prevNotebookId = useRef(notebook.id)
  useEffect(() => {
    const ids = notebook.cells.map((c) => c.id)
    const last = prevIds.current
    const switched = prevNotebookId.current !== notebook.id
    prevIds.current = ids
    prevNotebookId.current = notebook.id

    if (!autoScroll) return
    if (last === null || switched) {
      endRef.current?.scrollIntoView({ block: 'end' })
      return
    }
    const added = ids.filter((id) => !last.includes(id))
    if (added.length !== 1) return
    const id = added[0]
    if (ids[ids.length - 1] === id) {
      endRef.current?.scrollIntoView({ block: 'end' })
    } else {
      document
        .getElementById(`cell-${id}`)
        ?.scrollIntoView({ block: 'nearest' })
    }
  }, [notebook.cells, notebook.id, autoScroll])

  return (
    <div
      className="flex-1 overflow-y-auto px-4 py-4 sm:px-6"
      onContextMenu={(e) =>
        openContextMenu(e, [
          {
            label: 'Add code cell',
            onSelect: () => insertCell('end', 'math'),
          },
          {
            label: 'Add text cell',
            onSelect: () => insertCell('end', 'markdown'),
          },
          'divider',
          {
            label: 'Export as PDF…',
            disabled: count === 0,
            onSelect: () =>
              void exportNotebookPdf(notebook).catch((e) =>
                console.error('PDF export failed', e),
              ),
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
      {count > 0 && <CellInserter pos="start" />}
      {notebook.cells.map((c) => (
        <Fragment key={c.id}>
          <div id={`cell-${c.id}`}>
            <CellView cell={c} stale={stale.has(c.id)} />
          </div>
          <CellInserter pos={{ after: c.id }} />
        </Fragment>
      ))}
      <div ref={endRef} />
    </div>
  )
}

// A thin hover zone between rows: reveals "+ code" / "+ text" buttons that drop
// a fresh empty cell at this spot. The 1rem height also supplies the vertical
// rhythm between cells (the list dropped its `space-y` for this).
function CellInserter({ pos }: { pos: InsertPos }) {
  const insertCell = useNotebook((s) => s.insertCell)
  const btn =
    'rounded border border-edge bg-app px-1.5 text-[11px] leading-4 text-faint hover:border-edge-strong hover:text-ink'
  return (
    <div className="group/ins relative flex h-4 items-center justify-center">
      <div className="pointer-events-none absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-edge-strong opacity-0 transition-opacity group-hover/ins:opacity-50" />
      <div className="relative flex gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover/ins:opacity-100">
        <button
          type="button"
          title="insert a code cell here"
          onClick={() => insertCell(pos, 'math')}
          className={btn}
        >
          + code
        </button>
        <button
          type="button"
          title="insert a text cell here"
          onClick={() => insertCell(pos, 'markdown')}
          className={btn}
        >
          + text
        </button>
      </div>
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
        everything below) · hover between cells for <em>+ code</em> /{' '}
        <em>+ text</em> · text cells render <code>$LaTeX$</code> · notebooks
        save automatically
      </p>
    </div>
  )
}
