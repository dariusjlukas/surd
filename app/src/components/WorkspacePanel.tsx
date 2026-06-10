// The variables table: every workspace binding, refreshed after each
// successful evaluation. Values render as math; anything enormous (a
// 1000-digit integer, a huge matrix) falls back to truncated plain text
// rather than asking KaTeX to typeset a wall.

import type { WorkspaceEntry } from '../engine/types'
import { useNotebook } from '../state/store'
import { MathInline } from './MathOutput'

const MAX_RENDERED_CHARS = 120

export function WorkspacePanel() {
  const workspace = useNotebook((s) => s.workspace)

  return (
    <aside className="flex w-72 shrink-0 flex-col border-l border-slate-800">
      <div className="border-b border-slate-800 px-4 py-2 text-xs font-medium uppercase tracking-wide text-slate-500">
        workspace
      </div>
      <div className="flex-1 overflow-y-auto">
        {workspace.length === 0 ? (
          <p className="px-4 py-3 text-xs text-slate-600">
            no variables yet — try <code className="text-slate-500">x := 3</code>
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
    <tr className="border-b border-slate-800/60 align-baseline">
      <td className="w-1 whitespace-nowrap px-4 py-1.5 font-mono text-sky-300">
        {entry.name}
      </td>
      <td className="px-2 py-1.5 text-slate-300">
        <Value entry={entry} />
      </td>
    </tr>
  )
}

function Value({ entry }: { entry: WorkspaceEntry }) {
  if (entry.kind === 'function' || entry.text.length > MAX_RENDERED_CHARS) {
    return (
      <span className="break-all font-mono text-xs text-slate-400" title={entry.text}>
        {entry.text.length > MAX_RENDERED_CHARS
          ? entry.text.slice(0, MAX_RENDERED_CHARS) + '…'
          : entry.text}
      </span>
    )
  }
  return <MathInline latex={entry.latex} fallback={entry.text} />
}
