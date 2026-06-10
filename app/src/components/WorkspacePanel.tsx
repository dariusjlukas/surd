// The variables table: every workspace binding, refreshed after each
// successful evaluation. Values render as math; anything enormous (a
// 1000-digit integer, a huge matrix) falls back to truncated plain text
// rather than asking KaTeX to typeset a wall. Width is user-resizable
// (PaneResizer in App); right-click a row to copy the binding.

import type { WorkspaceEntry } from '../engine/types'
import { useNotebook } from '../state/store'
import { openContextMenu } from '../state/contextMenu'
import { MathInline } from './MathOutput'

const MAX_RENDERED_CHARS = 120

const copy = (text: string) => void navigator.clipboard.writeText(text)

export function WorkspacePanel({ width }: { width: number }) {
  const workspace = useNotebook((s) => s.workspace)

  return (
    <aside style={{ width }} className="flex shrink-0 flex-col border-l border-edge">
      <div className="border-b border-edge px-4 py-2 text-xs font-medium uppercase tracking-wide text-faint">
        workspace
      </div>
      <div className="flex-1 overflow-y-auto">
        {workspace.length === 0 ? (
          <p className="px-4 py-3 text-xs text-faint">
            no variables yet — try <code className="text-muted">x := 3</code>
          </p>
        ) : (
          <table className="w-full text-sm">
            <tbody>
              {workspace.map((entry) => (
                <Row key={entry.name} entry={entry} />
              ))}
            </tbody>
          </table>
        )}
      </div>
    </aside>
  )
}

function Row({ entry }: { entry: WorkspaceEntry }) {
  return (
    <tr
      className="border-b border-edge/60 align-baseline hover:bg-hover/30"
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: 'Copy name', onSelect: () => copy(entry.name) },
          { label: 'Copy value', onSelect: () => copy(entry.text) },
          {
            label: `Copy as assignment (${entry.name} := …)`,
            onSelect: () => copy(`${entry.name} := ${entry.text}`),
          },
        ])
      }
    >
      <td className="w-1 whitespace-nowrap px-4 py-1.5 font-mono text-accent">
        {entry.name}
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
      <span className="break-all font-mono text-xs text-muted" title={entry.text}>
        {entry.text.length > MAX_RENDERED_CHARS
          ? entry.text.slice(0, MAX_RENDERED_CHARS) + '…'
          : entry.text}
      </span>
    )
  }
  return <MathInline latex={entry.latex} fallback={entry.text} />
}
