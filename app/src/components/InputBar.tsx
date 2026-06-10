// REPL line discipline: lines accumulate while the block is syntactically
// incomplete (unclosed brackets / if…end), exactly like the native REPL.
// Incompleteness is decided by the engine's own lexer (main-thread wasm copy),
// not a JS re-implementation that could drift.

import { useRef, useState } from 'react'
import { is_blank, is_incomplete } from '../engine/lexer'
import { useInputHistory, useNotebook } from '../state/store'

export function InputBar() {
  const engineStatus = useNotebook((s) => s.engineStatus)
  const submit = useNotebook((s) => s.submit)
  const history = useInputHistory()

  const [line, setLine] = useState('')
  const [buffer, setBuffer] = useState<string[]>([])
  const recallRef = useRef(-1)

  const ready = engineStatus === 'ready'
  const continuing = buffer.length > 0

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      if (!ready) return
      const lines = [...buffer, line]
      const src = lines.join('\n')
      setLine('')
      recallRef.current = -1
      if (is_incomplete(src)) {
        setBuffer(lines)
        return
      }
      setBuffer([])
      if (!is_blank(src)) void submit(src)
    } else if (e.key === 'ArrowUp' && !continuing && line === '') {
      if (history.length === 0) return
      e.preventDefault()
      recallRef.current =
        recallRef.current < 0 ? history.length - 1 : Math.max(0, recallRef.current - 1)
      setLine(history[recallRef.current])
    } else if (e.key === 'ArrowDown' && recallRef.current >= 0) {
      e.preventDefault()
      recallRef.current += 1
      if (recallRef.current >= history.length) {
        recallRef.current = -1
        setLine('')
      } else {
        setLine(history[recallRef.current])
      }
    } else if (e.key === 'Escape' && continuing) {
      setBuffer([])
      setLine('')
    }
  }

  return (
    <div className="border-t border-slate-800 px-4 py-3 sm:px-6">
      {continuing && (
        <pre className="mb-1 whitespace-pre-wrap font-mono text-xs text-slate-500">
          {buffer.join('\n')}
        </pre>
      )}
      <div className="flex items-center gap-2">
        <span className="select-none font-mono text-sky-400">
          {continuing ? '..' : '>>'}
        </span>
        <input
          value={line}
          onChange={(e) => setLine(e.target.value)}
          onKeyDown={onKeyDown}
          disabled={engineStatus === 'failed'}
          placeholder={ready ? '' : 'engine loading…'}
          autoFocus
          autoComplete="off"
          spellCheck={false}
          className="flex-1 bg-transparent font-mono text-sm text-slate-100 outline-none placeholder:text-slate-600"
        />
      </div>
    </div>
  )
}
