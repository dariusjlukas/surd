// Notebook list: switch, create, rename (double-click, pen icon, or right-click),
// export, delete, and import at the foot. Rename is an inline input
// committed on Enter/blur, abandoned on Escape. Width is user-resizable
// (PaneResizer in App).

import { useRef, useState } from 'react'
import {
  faDownload,
  faPen,
  faPlus,
  faTrashCan,
} from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import { downloadNotebook, parseNotebookFile } from '../state/notebookFile'
import { exportNotebookPdf } from './exportPdf'
import { useSettings } from '../state/settings'
import { untitledName, useNotebook, type Notebook } from '../state/store'
import { openContextMenu } from '../state/contextMenu'

export function Sidebar({
  width,
  mobile,
}: {
  width: number
  /** Render as a fixed overlay drawer (phone layout) instead of a pinned,
   * resizable column. */
  mobile?: boolean
}) {
  const notebooks = useNotebook((s) => s.notebooks)
  const activeId = useNotebook((s) => s.activeId)
  const createNotebook = useNotebook((s) => s.createNotebook)
  const addNotebook = useNotebook((s) => s.addNotebook)
  const fileRef = useRef<HTMLInputElement>(null)
  const [importError, setImportError] = useState<string | null>(null)

  const onImportFile = async (file: File) => {
    try {
      const { name, cells } = parseNotebookFile(await file.text())
      // Imported names may collide with existing ones; suffix until unique.
      const taken = new Set(notebooks.map((n) => n.name))
      let unique = name
      for (let i = 2; taken.has(unique); i++) unique = `${name} (${i})`
      addNotebook(unique, cells)
      setImportError(null)
    } catch (e) {
      setImportError(e instanceof Error ? e.message : 'import failed')
    }
  }

  return (
    <aside
      style={mobile ? undefined : { width }}
      className={`flex flex-col border-r border-edge ${
        mobile
          ? 'fixed inset-y-0 left-0 z-40 w-[min(18rem,85vw)] bg-app shadow-xl'
          : 'shrink-0 bg-surface/40'
      }`}
    >
      <div className="flex items-center justify-between border-b border-edge px-3 py-2">
        <span className="text-xs font-medium uppercase tracking-wide text-faint">
          notebooks
        </span>
        <button
          onClick={createNotebook}
          title={`new notebook (${untitledName(notebooks)})`}
          className="rounded-md px-1.5 py-0.5 text-muted hover:bg-hover hover:text-ink"
        >
          <FontAwesomeIcon icon={faPlus} className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto py-1">
        {notebooks.map((nb) => (
          <NotebookRow key={nb.id} nb={nb} active={nb.id === activeId} />
        ))}
      </div>
      <div className="border-t border-edge p-2">
        {importError && (
          <p className="mb-1 px-1 text-xs text-danger">
            import failed: {importError}
          </p>
        )}
        <button
          onClick={() => fileRef.current?.click()}
          className="w-full rounded-md border border-edge px-2 py-1 text-xs text-muted hover:border-edge-strong hover:text-ink"
        >
          import notebook…
        </button>
        <input
          ref={fileRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={(e) => {
            const file = e.target.files?.[0]
            e.target.value = '' // allow re-importing the same file
            if (file) void onImportFile(file)
          }}
        />
      </div>
    </aside>
  )
}

function NotebookRow({ nb, active }: { nb: Notebook; active: boolean }) {
  const selectNotebook = useNotebook((s) => s.selectNotebook)
  const renameNotebook = useNotebook((s) => s.renameNotebook)
  const deleteNotebook = useNotebook((s) => s.deleteNotebook)
  const confirmDelete = useSettings((s) => s.confirmDelete)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(nb.name)

  const startRename = () => {
    setDraft(nb.name)
    setEditing(true)
  }
  const commitRename = () => {
    setEditing(false)
    renameNotebook(nb.id, draft)
  }
  const remove = () => {
    if (
      !confirmDelete ||
      window.confirm(`Delete "${nb.name}" and its workspace?`)
    ) {
      deleteNotebook(nb.id)
    }
  }

  if (editing) {
    return (
      <div className="px-2 py-0.5">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commitRename()
            else if (e.key === 'Escape') setEditing(false)
          }}
          autoFocus
          onFocus={(e) => e.target.select()}
          className="w-full rounded-md border border-accent/60 bg-app px-1.5 py-0.5 text-sm text-ink outline-none"
        />
      </div>
    )
  }

  return (
    <div
      role="button"
      tabIndex={0}
      aria-current={active ? 'page' : undefined}
      onClick={() => selectNotebook(nb.id)}
      onDoubleClick={startRename}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          selectNotebook(nb.id)
        }
      }}
      onContextMenu={(e) =>
        openContextMenu(e, [
          {
            label: 'Open',
            onSelect: () => selectNotebook(nb.id),
            disabled: active,
          },
          { label: 'Rename…', onSelect: startRename },
          { label: 'Export as JSON', onSelect: () => downloadNotebook(nb) },
          {
            label: 'Export as PDF…',
            onSelect: () =>
              void exportNotebookPdf(nb).catch((e) =>
                console.error('PDF export failed', e),
              ),
          },
          'divider',
          { label: 'Delete', onSelect: remove, danger: true },
        ])
      }
      title={`${nb.cells.length} cell${nb.cells.length === 1 ? '' : 's'} — double-click to rename`}
      className={`group mx-1 flex cursor-pointer items-center gap-1 rounded-md px-2 py-1.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/50 ${
        active
          ? 'bg-accent/10 font-medium text-accent'
          : 'text-muted hover:bg-hover/60 hover:text-ink'
      }`}
    >
      <span className="min-w-0 flex-1 truncate">{nb.name}</span>
      <RowButton label="rename" onClick={startRename}>
        <FontAwesomeIcon icon={faPen} className="h-2.5 w-2.5" />
      </RowButton>
      <RowButton label="export as .json" onClick={() => downloadNotebook(nb)}>
        <FontAwesomeIcon icon={faDownload} className="h-2.5 w-2.5" />
      </RowButton>
      <RowButton label="delete" onClick={remove}>
        <FontAwesomeIcon icon={faTrashCan} className="h-2.5 w-2.5" />
      </RowButton>
    </div>
  )
}

function RowButton({
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
      onClick={(e) => {
        e.stopPropagation() // don't also select the notebook
        onClick()
      }}
      className="nb-row-action hidden rounded px-1 text-xs text-faint hover:bg-hover hover:text-ink group-hover:block"
    >
      {children}
    </button>
  )
}
