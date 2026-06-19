// One notebook cell. Math cells show source + result; a single click drops
// the source into an in-place editor with the input bar's line discipline.
// Editing without re-running leaves the shown result STALE — and not only this
// cell's: any later cell that reads what the edit changes is stale too (see
// state/staleness). Stale cells are flagged; the edited one keeps its editor
// open until you re-run (enter, recomputing the cell and everything below it;
// see store.recomputeFrom) or revert (esc). Markdown cells render sanitized
// HTML and double-click into a plain editor. Committing an empty source
// deletes the cell. Both kinds carry a right-click menu mirroring the hover
// buttons.

import {
  lazy,
  memo,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { faXmark } from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import DOMPurify from 'dompurify'
import { marked } from 'marked'
import { insertNewlineAndIndent } from '@codemirror/commands'
import type { KeyBinding } from '@codemirror/view'
import { CodeEditor, type CodeEditorHandle } from '../editor/CodeEditor'
import { is_incomplete } from '../engine/lexer'
import { useNotebook, type Cell } from '../state/store'
import { useDrafts } from '../state/staleness'
import { openContextMenu, type MenuEntry } from '../state/contextMenu'
import { MathOutput } from './MathOutput'

// ThreeJS is the heaviest dependency; load it only when a plot first renders.
const PlotView = lazy(() =>
  import('../plot/PlotView').then((m) => ({ default: m.PlotView })),
)
const Surface3DView = lazy(() =>
  import('../plot/Surface3DView').then((m) => ({ default: m.Surface3DView })),
)

// Memoized so a keystroke in one cell (which re-renders NotebookView to
// recompute the stale set) only re-renders cells whose `stale` flag flips.
export const CellView = memo(function CellView({
  cell,
  stale,
}: {
  cell: Cell
  /** Result is out of date w.r.t. an in-progress edit (this cell's or an
   * upstream one it depends on). Only meaningful for math cells. */
  stale: boolean
}) {
  if (cell.kind === 'markdown') return <MarkdownCell cell={cell} />
  if (cell.kind === 'data') return <DataCell cell={cell} />
  return <MathCell cell={cell} stale={stale} />
})

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

function MathCell({ cell, stale }: { cell: Cell; stale: boolean }) {
  const rerun = useNotebook((s) => s.rerun)
  const insertCell = useNotebook((s) => s.insertCell)
  const deleteCell = useNotebook((s) => s.deleteCell)
  const updateCell = useNotebook((s) => s.updateCell)
  const ready = useNotebook((s) => s.engineStatus === 'ready')
  // Actions are stable refs, so subscribing here never forces a re-render.
  const setDraft = useDrafts((s) => s.setDraft)
  const clearDraft = useDrafts((s) => s.clearDraft)

  const [editing, setEditing] = useState(false)
  const editorRef = useRef<CodeEditorHandle>(null)

  // Drop any pending draft if the cell unmounts (deleted, notebook switched).
  useEffect(() => () => clearDraft(cell.id), [cell.id, clearDraft])

  const open = () => setEditing(true)
  const close = () => {
    setEditing(false)
    clearDraft(cell.id)
  }
  const commit = () => {
    const src = editorRef.current?.get() ?? ''
    close()
    if (src === cell.src) return // nothing to re-run
    if (src.trim() === '') void deleteCell(cell.id)
    else void updateCell(cell.id, src) // writes src + recomputes from here down
  }
  const revert = () => {
    editorRef.current?.set(cell.src)
    close()
  }
  // Leaving an untouched cell collapses it back to the read view; one with
  // un-run edits stays open so its stale marker persists. Read the draft
  // imperatively so per-keystroke edits don't re-render the cell.
  const onBlur = () => {
    const draft = useDrafts.getState().drafts[cell.id]
    if (draft === undefined || draft === cell.src) close()
  }
  // The "run" control: apply this cell's pending edits if any, else re-run
  // from here as-is (which is how you refresh a downstream-stale cell — by
  // re-running the edited cell above it).
  const runFromHere = () => {
    const src = editorRef.current?.get()
    if (src !== undefined && src !== cell.src) commit()
    else void rerun(cell.id)
  }

  const keys: KeyBinding[] = [
    {
      key: 'Enter',
      run: (view) => {
        if (is_incomplete(view.state.doc.toString())) {
          return insertNewlineAndIndent(view)
        }
        if (!ready) return true // swallow: engine busy, keep the edit pending
        commit()
        return true
      },
    },
    { key: 'Shift-Enter', run: (view) => insertNewlineAndIndent(view) },
    { key: 'Mod-Enter', run: () => (ready && commit(), true) },
    { key: 'Escape', run: () => (revert(), true) },
  ]

  const r = cell.result
  const menu: MenuEntry[] = [
    { label: 'Edit', onSelect: open },
    {
      label: stale ? 'Re-run with edits' : 'Run from here',
      onSelect: runFromHere,
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
      className={`group -mx-2 rounded-md px-2 py-1 hover:bg-surface/50 ${
        editing
          ? `bg-surface/60 ring-1 ring-inset ${
              stale ? 'ring-warn/40' : 'ring-accent/40'
            }`
          : ''
      }`}
      onContextMenu={(e) => openContextMenu(e, menu)}
    >
      <div className="flex items-start gap-2">
        {editing ? (
          <>
            <span className="select-none pt-0.5 font-mono text-sm text-accent">
              &gt;&gt;
            </span>
            <CodeEditor
              ref={editorRef}
              initialDoc={cell.src}
              autoFocus
              keys={keys}
              onDocChange={(doc) => setDraft(cell.id, doc)}
              onBlur={onBlur}
            />
          </>
        ) : (
          <pre
            onClick={open}
            className="min-w-0 flex-1 cursor-text whitespace-pre-wrap font-mono text-sm text-muted"
          >
            <span className="select-none text-accent">&gt;&gt; </span>
            {cell.src}
          </pre>
        )}
        <span className="invisible flex shrink-0 gap-1 group-hover:visible">
          {ready && (
            <CellButton
              label={
                stale
                  ? 're-run with your edits (this cell and everything below)'
                  : 're-evaluate this cell and everything below'
              }
              onClick={runFromHere}
            >
              run
            </CellButton>
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
      {editing && (
        <div
          className={`pl-6 pt-0.5 text-[11px] ${
            stale ? 'text-warn' : 'text-faint'
          }`}
        >
          {stale
            ? 'stale — enter re-runs from here · esc reverts'
            : 'enter evaluates from here down · esc cancels'}
        </div>
      )}
      <div className="pl-6">
        {stale && cell.status === 'done' && (
          <div className="mb-0.5 text-[11px] font-medium text-warn">
            {editing ? '↺ stale output' : '↺ stale — depends on an unrun edit'}
          </div>
        )}
        <div className={stale ? 'opacity-50' : undefined}>
          <Output cell={cell} />
        </div>
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
