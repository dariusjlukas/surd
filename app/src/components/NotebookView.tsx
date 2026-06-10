import { useEffect, useRef } from 'react'
import { useNotebook } from '../state/store'
import { CellView } from './CellView'

export function NotebookView() {
  const cells = useNotebook((s) => s.cells)
  const endRef = useRef<HTMLDivElement>(null)
  const count = cells.length

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: 'end' })
  }, [count])

  return (
    <div className="flex-1 space-y-4 overflow-y-auto px-4 py-4 sm:px-6">
      {count === 0 && <Welcome />}
      {cells.map((c) => (
        <CellView key={c.id} cell={c} />
      ))}
      <div ref={endRef} />
    </div>
  )
}

function Welcome() {
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
    <div className="text-sm text-slate-500">
      <p className="mb-2">
        Exact by default: <code className="text-slate-400">1/3</code> stays a third,{' '}
        <code className="text-slate-400">sqrt(2)</code> stays a radical. Floats only via{' '}
        <code className="text-slate-400">N(x)</code>. Try:
      </p>
      <ul className="space-y-1 font-mono">
        {examples.map((e) => (
          <li key={e} className="text-slate-400">
            {e}
          </li>
        ))}
      </ul>
    </div>
  )
}
