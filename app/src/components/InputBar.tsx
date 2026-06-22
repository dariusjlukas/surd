// The notebook's primary input: a CodeMirror editor with the REPL's line
// discipline. Enter submits when the block is syntactically complete and
// inserts a newline while it isn't (unclosed brackets / if…end) — decided by
// the engine's own lexer, not a JS re-implementation that could drift.
// Shift+Enter always inserts a newline; ↑/↓ recall history when the editor
// is empty; Escape clears.

import { useRef } from 'react'
import { insertNewlineAndIndent } from '@codemirror/commands'
import type { KeyBinding } from '@codemirror/view'
import { CodeEditor, type CodeEditorHandle } from '../editor/CodeEditor'
import { is_blank, is_incomplete } from '../engine/lexer'
import { useInputHistory, useNotebook } from '../state/store'

export function InputBar() {
  const engineStatus = useNotebook((s) => s.engineStatus)
  const submit = useNotebook((s) => s.submit)
  const insertCell = useNotebook((s) => s.insertCell)
  const history = useInputHistory()

  const editorRef = useRef<CodeEditorHandle>(null)
  const recallRef = useRef(-1)
  // set() fires onDocChange too; recall must survive its own writes.
  const settingRef = useRef(false)
  const setDoc = (doc: string) => {
    settingRef.current = true
    editorRef.current?.set(doc)
    settingRef.current = false
  }

  const ready = engineStatus === 'ready'

  const keys: KeyBinding[] = [
    {
      key: 'Enter',
      run: (view) => {
        const src = view.state.doc.toString()
        if (is_incomplete(src)) return insertNewlineAndIndent(view)
        if (!ready) return true // swallow: engine busy, keep the draft
        setDoc('')
        recallRef.current = -1
        if (!is_blank(src)) void submit(src)
        return true
      },
    },
    { key: 'Shift-Enter', run: (view) => insertNewlineAndIndent(view) },
    {
      key: 'ArrowUp',
      run: (view) => {
        if (view.state.doc.length > 0 && recallRef.current < 0) return false
        if (history.length === 0) return false
        recallRef.current =
          recallRef.current < 0
            ? history.length - 1
            : Math.max(0, recallRef.current - 1)
        setDoc(history[recallRef.current])
        return true
      },
    },
    {
      key: 'ArrowDown',
      run: () => {
        if (recallRef.current < 0) return false
        recallRef.current += 1
        if (recallRef.current >= history.length) {
          recallRef.current = -1
          setDoc('')
        } else {
          setDoc(history[recallRef.current])
        }
        return true
      },
    },
    {
      key: 'Escape',
      run: () => {
        recallRef.current = -1
        setDoc('')
        return true
      },
    },
  ]

  return (
    <div className="border-t border-edge bg-surface/30 px-4 py-3 sm:px-6">
      <div className="flex items-start gap-2">
        {/* Match the editor's text metrics (CodeMirror renders at 14px /
            line-height 1.4, with 2px top padding on .cm-content) so the prompt's
            first-line box lines up with the input text under items-start. */}
        <span className="select-none pt-0.5 font-mono text-sm leading-[1.4] text-accent">
          &gt;&gt;
        </span>
        <CodeEditor
          ref={editorRef}
          placeholder={ready ? '' : 'engine loading…'}
          autoFocus
          keys={keys}
          onDocChange={() => {
            // typing invalidates the recall cursor (programmatic sets don't)
            if (!settingRef.current) recallRef.current = -1
          }}
        />
        <button
          onClick={() => insertCell(null, 'markdown')}
          title="add a text (markdown) cell"
          className="shrink-0 rounded-md border border-edge px-2 py-0.5 text-xs text-faint hover:border-edge-strong hover:text-ink"
        >
          + note
        </button>
      </div>
    </div>
  )
}
