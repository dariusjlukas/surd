// The variables table: every workspace binding, refreshed after each
// successful evaluation. Values render as math; anything enormous (a
// 1000-digit integer, a huge matrix) falls back to truncated plain text
// rather than asking KaTeX to typeset a wall. Width is user-resizable
// (PaneResizer in App); right-click a row to copy or export the binding.
//
// Raw data lives here too: "import" reads a file (surd-data JSON, generic
// JSON, or CSV) into a fresh variable — named members arrive inside a struct
// so they can't collide with existing bindings — and "export" toggles a
// selection mode that saves any group of variables into one surd-data file.

import { useRef, useState } from 'react'
import {
  faDownload,
  faUpload,
  faWaveSquare,
} from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import type { ImportFormat, WorkspaceEntry } from '../engine/types'
import { nameToLatex } from '../engine/nameLatex'
import { useNotebook } from '../state/store'
import { downloadDataFile } from '../state/dataFile'
import { openContextMenu } from '../state/contextMenu'
import { MathInline } from './MathOutput'

const MAX_RENDERED_CHARS = 120

const copy = (text: string) => void navigator.clipboard.writeText(text)

/** Chunked base64 encode — String.fromCharCode(...) overflows the argument
 * limit on multi-megabyte buffers. */
function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000
  let bin = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  }
  return btoa(bin)
}

export function WorkspacePanel({
  width,
  mobile,
}: {
  width: number
  /** Render as a fixed overlay drawer (phone layout) instead of a pinned,
   * resizable column. */
  mobile?: boolean
}) {
  const workspace = useNotebook((s) => s.workspace)
  const importData = useNotebook((s) => s.importData)
  const exportData = useNotebook((s) => s.exportData)
  const ready = useNotebook((s) => s.engineStatus === 'ready')
  const fileRef = useRef<HTMLInputElement>(null)
  const signalFileRef = useRef<HTMLInputElement>(null)
  const [selecting, setSelecting] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [error, setError] = useState<string | null>(null)

  const onImportFile = async (file: File) => {
    try {
      await importData(file.name, await file.text())
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'import failed')
    }
  }

  /** Bulk signal imports: the format follows the extension. Binary payloads
   * ride the transcript as base64 (see ImportFormat). */
  const onImportSignalFile = async (file: File) => {
    const ext = (file.name.split('.').pop() ?? '').toLowerCase()
    const format: ImportFormat | null =
      ext === 'wav'
        ? 'wav'
        : ext === 'f64'
          ? 'raw-f64'
          : ext === 'f32'
            ? 'raw-f32'
            : ext === 'i16' || ext === 'pcm'
              ? 'raw-i16'
              : ext === 'csv' || ext === 'tsv' || ext === 'txt'
                ? 'csv-packed'
                : null
    if (format === null) {
      setError(
        `cannot infer the sample format of '.${ext}' — use .wav, .csv, or ` +
          'rename raw binary with its sample type: .f64 .f32 .i16',
      )
      return
    }
    try {
      const payload =
        format === 'csv-packed'
          ? await file.text()
          : bytesToBase64(new Uint8Array(await file.arrayBuffer()))
      await importData(file.name, payload, format)
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'import failed')
    }
  }

  const doExport = async (names: string[], baseName: string) => {
    try {
      downloadDataFile(await exportData(names), baseName)
      setError(null)
      setSelecting(false)
      setSelected(new Set())
    } catch (e) {
      setError(e instanceof Error ? e.message : 'export failed')
    }
  }

  const toggle = (name: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })

  return (
    <aside
      style={mobile ? undefined : { width }}
      className={`flex flex-col border-l border-edge ${
        mobile
          ? 'fixed inset-y-0 right-0 z-40 w-[min(20rem,85vw)] bg-app shadow-xl'
          : 'shrink-0'
      }`}
    >
      <div className="flex items-center justify-between border-b border-edge px-4 py-2">
        <span className="text-xs font-medium uppercase tracking-wide text-faint">
          workspace
        </span>
        <span className="flex gap-1">
          <button
            title="import raw data (surd-data JSON, generic JSON, or CSV) into a variable"
            disabled={!ready}
            onClick={() => fileRef.current?.click()}
            className="rounded-md px-1.5 py-0.5 text-muted hover:bg-hover hover:text-ink disabled:opacity-40"
          >
            <FontAwesomeIcon icon={faUpload} className="h-3 w-3" />
          </button>
          <button
            title="import a signal (WAV audio, raw .f64/.f32/.i16 binary, or large CSV) as certified bulk data"
            disabled={!ready}
            onClick={() => signalFileRef.current?.click()}
            className="rounded-md px-1.5 py-0.5 text-muted hover:bg-hover hover:text-ink disabled:opacity-40"
          >
            <FontAwesomeIcon icon={faWaveSquare} className="h-3 w-3" />
          </button>
          <button
            title="export variables to a data file"
            disabled={workspace.length === 0}
            onClick={() => {
              setSelecting((s) => !s)
              setSelected(new Set())
            }}
            className={`rounded-md px-1.5 py-0.5 hover:bg-hover hover:text-ink disabled:opacity-40 ${
              selecting ? 'bg-accent/10 text-accent' : 'text-muted'
            }`}
          >
            <FontAwesomeIcon icon={faDownload} className="h-3 w-3" />
          </button>
        </span>
        <input
          ref={fileRef}
          type="file"
          accept=".json,.csv,.tsv,.txt,application/json,text/csv"
          className="hidden"
          onChange={(e) => {
            const file = e.target.files?.[0]
            e.target.value = '' // allow re-importing the same file
            if (file) void onImportFile(file)
          }}
        />
        <input
          ref={signalFileRef}
          type="file"
          accept=".wav,.csv,.tsv,.txt,.f64,.f32,.i16,.pcm,audio/wav"
          className="hidden"
          onChange={(e) => {
            const file = e.target.files?.[0]
            e.target.value = ''
            if (file) void onImportSignalFile(file)
          }}
        />
      </div>
      {error && (
        <p className="border-b border-edge px-4 py-2 text-xs text-danger">
          {error}
        </p>
      )}
      <div className="flex-1 overflow-y-auto">
        {workspace.length === 0 ? (
          <p className="px-4 py-3 text-xs text-faint">
            no variables yet — try <code className="text-muted">x := 3</code> or
            import a data file
          </p>
        ) : (
          <table className="w-full text-sm">
            <tbody>
              {workspace.map((entry) => (
                <Row
                  key={entry.name}
                  entry={entry}
                  selecting={selecting}
                  selected={selected.has(entry.name)}
                  onToggle={() => toggle(entry.name)}
                  onExportOne={() => void doExport([entry.name], entry.name)}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>
      {selecting && (
        <div className="flex items-center gap-2 border-t border-edge px-3 py-2">
          <button
            onClick={() =>
              setSelected((prev) =>
                prev.size === workspace.length
                  ? new Set()
                  : new Set(workspace.map((w) => w.name)),
              )
            }
            className="rounded-md border border-edge px-2 py-1 text-xs text-muted hover:border-edge-strong hover:text-ink"
          >
            {selected.size === workspace.length ? 'none' : 'all'}
          </button>
          <button
            disabled={selected.size === 0}
            onClick={() => void doExport([...selected], 'workspace')}
            className="flex-1 rounded-md border border-edge px-2 py-1 text-xs text-muted hover:border-edge-strong hover:text-ink disabled:opacity-40"
          >
            export {selected.size || ''} selected…
          </button>
        </div>
      )}
    </aside>
  )
}

function Row({
  entry,
  selecting,
  selected,
  onToggle,
  onExportOne,
}: {
  entry: WorkspaceEntry
  selecting: boolean
  selected: boolean
  onToggle: () => void
  onExportOne: () => void
}) {
  return (
    <tr
      className="border-b border-edge/60 align-baseline hover:bg-hover/30"
      onClick={selecting ? onToggle : undefined}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: 'Copy name', onSelect: () => copy(entry.name) },
          { label: 'Copy value', onSelect: () => copy(entry.text) },
          {
            label: `Copy as assignment (${entry.name} := …)`,
            onSelect: () => copy(`${entry.name} := ${entry.text}`),
          },
          'divider',
          { label: 'Export variable…', onSelect: onExportOne },
        ])
      }
    >
      {selecting && (
        <td className="w-1 pl-3">
          <input
            type="checkbox"
            checked={selected}
            readOnly
            className="accent-current"
          />
        </td>
      )}
      <td className="w-1 whitespace-nowrap px-4 py-1.5 text-accent">
        <MathInline latex={nameToLatex(entry.name)} fallback={entry.name} />
      </td>
      <td className="px-2 py-1.5 text-muted">
        <Value entry={entry} />
      </td>
    </tr>
  )
}

function Value({ entry }: { entry: WorkspaceEntry }) {
  if (entry.kind === 'function' || entry.text.length > MAX_RENDERED_CHARS) {
    return (
      <span
        className="break-all font-mono text-xs text-muted"
        title={entry.text}
      >
        {entry.text.length > MAX_RENDERED_CHARS
          ? entry.text.slice(0, MAX_RENDERED_CHARS) + '…'
          : entry.text}
      </span>
    )
  }
  return <MathInline latex={entry.latex} fallback={entry.text} />
}
