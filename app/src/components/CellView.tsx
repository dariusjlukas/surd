// One notebook cell. Math cells show source + result and edit in place with
// the same line discipline as the input bar; committing an edit recomputes
// the cell and everything below it (see store.recomputeFrom). Markdown cells
// render sanitized HTML and double-click into a plain editor. Committing an
// empty source deletes the cell. Both kinds carry a right-click menu
// mirroring the hover buttons.

import { lazy, Suspense, useMemo, useRef, useState } from 'react'
import { faXmark } from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import DOMPurify from 'dompurify'
import { marked } from 'marked'
import { insertNewlineAndIndent } from '@codemirror/commands'
import type { KeyBinding } from '@codemirror/view'
import { CodeEditor, type CodeEditorHandle } from '../editor/CodeEditor'
import { is_incomplete } from '../engine/lexer'
import { useNotebook, type Cell } from '../state/store'
import { openContextMenu, type MenuEntry } from '../state/contextMenu'
import { MathOutput } from './MathOutput'

// ThreeJS is the heaviest dependency; load it only when a plot first renders.
const PlotView = lazy(() =>
  import('../plot/PlotView').then((m) => ({ default: m.PlotView })),
)
const Surface3DView = lazy(() =>
  import('../plot/Surface3DView').then((m) => ({ default: m.Surface3DView })),
)

export function CellView({ cell }: { cell: Cell }) {
  if (cell.kind === 'markdown') return <MarkdownCell cell={cell} />
  if (cell.kind === 'data') return <DataCell cell={cell} />
  return <MathCell cell={cell} />
}

// ---------------------------------------------------------------------------
// shared bits
// ---------------------------------------------------------------------------

/** Editing state + commit policy shared by both cell kinds: empty commits
 * delete the cell; unchanged commits just close the editor. */
function useCellEditing(cell: Cell) {
  const updateCell = useNotebook((s) => s.updateCell)
  const deleteCell = useNotebook((s) => s.deleteCell)
  // A freshly inserted cell (src === '') opens straight into edit mode.
  const [editing, setEditing] = useState(cell.src === '')
  const editorRef = useRef<CodeEditorHandle>(null)

  const commit = () => {
    const src = editorRef.current?.get() ?? ''
    setEditing(false)
    if (src.trim() === '') {
      void deleteCell(cell.id)
    } else if (src !== cell.src) {
      void updateCell(cell.id, src)
    }
  }
  const cancel = () => {
    setEditing(false)
    if (cell.src === '') void deleteCell(cell.id) // never-committed insert
  }
  return { editing, setEditing, editorRef, commit, cancel }
}

function CellButton({
  label,
  onClick,
  children,
}: {
  label: string
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      title={label}
      onClick={onClick}
      className="rounded border border-edge px-1.5 text-xs text-faint hover:border-edge-strong hover:text-ink"
    >
      {children}
    </button>
  )
}

function InsertNoteButton({ afterId }: { afterId: string }) {
  const insertCell = useNotebook((s) => s.insertCell)
  return (
    <CellButton
      label="insert text cell below"
      onClick={() => insertCell(afterId, 'markdown')}
    >
      +note
    </CellButton>
  )
}

const copy = (text: string) => void navigator.clipboard.writeText(text)

// ---------------------------------------------------------------------------
// math cells
// ---------------------------------------------------------------------------

function MathCell({ cell }: { cell: Cell }) {
  const rerun = useNotebook((s) => s.rerun)
  const insertCell = useNotebook((s) => s.insertCell)
  const deleteCell = useNotebook((s) => s.deleteCell)
  const ready = useNotebook((s) => s.engineStatus === 'ready')
  const { editing, setEditing, editorRef, commit, cancel } =
    useCellEditing(cell)

  if (editing) {
    const keys: KeyBinding[] = [
      {
        key: 'Enter',
        run: (view) => {
          if (is_incomplete(view.state.doc.toString())) {
            return insertNewlineAndIndent(view)
          }
          commit()
          return true
        },
      },
      { key: 'Shift-Enter', run: (view) => insertNewlineAndIndent(view) },
      { key: 'Mod-Enter', run: () => (commit(), true) },
      { key: 'Escape', run: () => (cancel(), true) },
    ]
    return (
      <div className="-mx-2 rounded-md border border-accent/50 bg-surface/60 px-2 py-1">
        <div className="flex items-start gap-2">
          <span className="select-none pt-0.5 font-mono text-sm text-accent">
            &gt;&gt;
          </span>
          <CodeEditor
            ref={editorRef}
            initialDoc={cell.src}
            autoFocus
            keys={keys}
          />
        </div>
        <div className="pl-6 pt-0.5 text-[11px] text-faint">
          enter evaluates from here down · esc cancels
        </div>
      </div>
    )
  }

  const r = cell.result
  const menu: MenuEntry[] = [
    { label: 'Edit', onSelect: () => setEditing(true), disabled: !ready },
    {
      label: 'Run from here',
      onSelect: () => void rerun(cell.id),
      disabled: !ready,
    },
    'divider',
    { label: 'Copy input', onSelect: () => copy(cell.src) },
    ...(r?.ok
      ? ([
          { label: 'Copy result as text', onSelect: () => copy(r.text) },
          { label: 'Copy result as LaTeX', onSelect: () => copy(r.latex) },
        ] satisfies MenuEntry[])
      : []),
    'divider',
    {
      label: 'Add note below',
      onSelect: () => insertCell(cell.id, 'markdown'),
    },
    {
      label: 'Delete cell',
      onSelect: () => void deleteCell(cell.id),
      danger: true,
      disabled: !ready,
    },
  ]

  return (
    <div
      className="group -mx-2 rounded-md px-2 py-1 hover:bg-surface/50"
      onContextMenu={(e) => openContextMenu(e, menu)}
    >
      <div className="flex items-start gap-2">
        <pre
          onDoubleClick={() => ready && setEditing(true)}
          className="min-w-0 flex-1 cursor-text whitespace-pre-wrap font-mono text-sm text-muted"
        >
          <span className="select-none text-accent">&gt;&gt; </span>
          {cell.src}
        </pre>
        <span className="invisible flex shrink-0 gap-1 group-hover:visible">
          {ready && (
            <>
              <CellButton
                label="edit (double-click also works)"
                onClick={() => setEditing(true)}
              >
                edit
              </CellButton>
              <CellButton
                label="re-evaluate this cell and everything below"
                onClick={() => void rerun(cell.id)}
              >
                run
              </CellButton>
            </>
          )}
          {r?.ok && (
            <>
              <CellButton
                label="copy result as plain text"
                onClick={() => copy(r.text)}
              >
                txt
              </CellButton>
              <CellButton
                label="copy result as LaTeX"
                onClick={() => copy(r.latex)}
              >
                tex
              </CellButton>
            </>
          )}
          <InsertNoteButton afterId={cell.id} />
          <DeleteButton cell={cell} />
        </span>
      </div>
      <div className="pl-6">
        <Output cell={cell} />
      </div>
    </div>
  )
}

function DeleteButton({ cell }: { cell: Cell }) {
  const deleteCell = useNotebook((s) => s.deleteCell)
  const ready = useNotebook((s) => s.engineStatus === 'ready')
  // Math and data cells affect the workspace; deleting them needs the engine.
  if (!ready && cell.kind !== 'markdown') return null
  return (
    <CellButton label="delete cell" onClick={() => void deleteCell(cell.id)}>
      <FontAwesomeIcon icon={faXmark} className="h-3 w-3" />
    </CellButton>
  )
}

function Output({ cell }: { cell: Cell }) {
  if (cell.status === 'pending') {
    return <div className="animate-pulse text-sm text-faint">evaluating…</div>
  }
  if (cell.status === 'cancelled') {
    return <div className="text-sm text-danger/80">cancelled</div>
  }
  const r = cell.result
  if (!r) return null
  if (!r.ok) {
    return <div className="font-mono text-sm text-danger">error: {r.error}</div>
  }
  switch (r.kind) {
    case 'plot':
      return r.plot ? (
        <Suspense
          fallback={
            <div className="h-80 max-w-2xl animate-pulse rounded-lg bg-surface" />
          }
        >
          <PlotView plot={r.plot} />
        </Suspense>
      ) : null
    case 'plot3d':
      return r.plot3d ? (
        <Suspense
          fallback={
            <div className="h-80 max-w-2xl animate-pulse rounded-lg bg-surface" />
          }
        >
          <Surface3DView plot={r.plot3d} />
        </Suspense>
      ) : null
    case 'function':
    case 'data':
      // value descriptions ("<function(n)>", import summaries), not math
      return <div className="font-mono text-sm text-muted">{r.text}</div>
    default:
      return <MathOutput latex={r.latex} fallback={r.text} />
  }
}

// ---------------------------------------------------------------------------
// data cells (raw-data imports)
// ---------------------------------------------------------------------------

/** A raw-data import. The payload isn't editable — re-import the file
 * instead — so this is a plain row: label, summary, run/delete. */
function DataCell({ cell }: { cell: Cell }) {
  const rerun = useNotebook((s) => s.rerun)
  const insertCell = useNotebook((s) => s.insertCell)
  const deleteCell = useNotebook((s) => s.deleteCell)
  const ready = useNotebook((s) => s.engineStatus === 'ready')

  const menu: MenuEntry[] = [
    {
      label: 'Run from here',
      onSelect: () => void rerun(cell.id),
      disabled: !ready,
    },
    {
      label: `Copy variable name (${cell.dataName ?? ''})`,
      onSelect: () => copy(cell.dataName ?? ''),
    },
    {
      label: 'Copy imported file contents',
      onSelect: () => copy(cell.dataPayload ?? ''),
    },
    'divider',
    {
      label: 'Add note below',
      onSelect: () => insertCell(cell.id, 'markdown'),
    },
    {
      label: 'Delete cell',
      onSelect: () => void deleteCell(cell.id),
      danger: true,
      disabled: !ready,
    },
  ]

  return (
    <div
      className="group -mx-2 rounded-md px-2 py-1 hover:bg-surface/50"
      onContextMenu={(e) => openContextMenu(e, menu)}
    >
      <div className="flex items-start gap-2">
        <pre className="min-w-0 flex-1 whitespace-pre-wrap font-mono text-sm text-muted">
          <span className="select-none text-accent">⇣ </span>
          {cell.src}
        </pre>
        <span className="invisible flex shrink-0 gap-1 group-hover:visible">
          {ready && (
            <CellButton
              label="re-import this data and re-evaluate everything below"
              onClick={() => void rerun(cell.id)}
            >
              run
            </CellButton>
          )}
          <InsertNoteButton afterId={cell.id} />
          <DeleteButton cell={cell} />
        </span>
      </div>
      <div className="pl-6">
        <Output cell={cell} />
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// markdown cells
// ---------------------------------------------------------------------------

function MarkdownCell({ cell }: { cell: Cell }) {
  const insertCell = useNotebook((s) => s.insertCell)
  const deleteCell = useNotebook((s) => s.deleteCell)
  const { editing, setEditing, editorRef, commit, cancel } =
    useCellEditing(cell)

  if (editing) {
    const keys: KeyBinding[] = [
      { key: 'Shift-Enter', run: () => (commit(), true) },
      { key: 'Mod-Enter', run: () => (commit(), true) },
      { key: 'Escape', run: () => (cancel(), true) },
    ]
    return (
      <div className="-mx-2 rounded-md border border-edge-strong bg-surface/60 px-2 py-1">
        <CodeEditor
          ref={editorRef}
          initialDoc={cell.src}
          lang="plain"
          placeholder="markdown — *italic*, **bold**, # heading, `code`"
          autoFocus
          keys={keys}
        />
        <div className="pt-0.5 text-[11px] text-faint">
          shift+enter renders · esc cancels
        </div>
      </div>
    )
  }

  return (
    <div
      className="group -mx-2 rounded-md px-2 py-1 hover:bg-surface/50"
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: 'Edit', onSelect: () => setEditing(true) },
          { label: 'Copy source', onSelect: () => copy(cell.src) },
          'divider',
          {
            label: 'Add note below',
            onSelect: () => insertCell(cell.id, 'markdown'),
          },
          {
            label: 'Delete cell',
            onSelect: () => void deleteCell(cell.id),
            danger: true,
          },
        ])
      }
    >
      <div className="flex items-start gap-2">
        <div
          onDoubleClick={() => setEditing(true)}
          className="min-w-0 flex-1 cursor-text"
        >
          <MarkdownView src={cell.src} />
        </div>
        <span className="invisible flex shrink-0 gap-1 group-hover:visible">
          <CellButton
            label="edit (double-click also works)"
            onClick={() => setEditing(true)}
          >
            edit
          </CellButton>
          <InsertNoteButton afterId={cell.id} />
          <DeleteButton cell={cell} />
        </span>
      </div>
    </div>
  )
}

function MarkdownView({ src }: { src: string }) {
  const html = useMemo(
    () => DOMPurify.sanitize(marked.parse(src, { async: false })),
    [src],
  )
  return (
    <div
      className="md-cell text-sm"
      dangerouslySetInnerHTML={{ __html: html }}
    />
  )
}
