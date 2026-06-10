import { lazy, Suspense } from 'react'
import type { Cell } from '../state/store'
import { MathOutput } from './MathOutput'

// ThreeJS is the heaviest dependency; load it only when a plot first renders.
const PlotView = lazy(() =>
  import('../plot/PlotView').then((m) => ({ default: m.PlotView })),
)

export function CellView({ cell }: { cell: Cell }) {
  return (
    <div className="group">
      <pre className="whitespace-pre-wrap font-mono text-sm text-slate-400">
        <span className="select-none text-sky-400">&gt;&gt; </span>
        {cell.src}
      </pre>
      <div className="pl-6">
        <Output cell={cell} />
      </div>
    </div>
  )
}

function Output({ cell }: { cell: Cell }) {
  if (cell.status === 'pending') {
    return <div className="animate-pulse text-sm text-slate-500">evaluating…</div>
  }
  if (cell.status === 'cancelled') {
    return <div className="text-sm text-rose-400/80">cancelled</div>
  }
  const r = cell.result
  if (!r) return null
  if (!r.ok) {
    return <div className="font-mono text-sm text-rose-400">error: {r.error}</div>
  }
  switch (r.kind) {
    case 'plot':
      return r.plot ? (
        <Suspense
          fallback={<div className="h-80 max-w-2xl animate-pulse rounded-lg bg-slate-900" />}
        >
          <PlotView plot={r.plot} />
        </Suspense>
      ) : null
    case 'function':
      // "<function(n)>" is a value description, not math
      return <div className="font-mono text-sm text-slate-300">{r.text}</div>
    default:
      return <MathOutput latex={r.latex} fallback={r.text} />
  }
}
