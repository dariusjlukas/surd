// Thin React wrapper around a CodeMirror EditorView. The view is created
// once; parents that need to read/set/clear the document do it through the
// imperative handle (the input bar's submit-and-clear and history recall
// don't fit a controlled-component shape).

import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
} from 'react'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { completionKeymap } from '@codemirror/autocomplete'
import { Compartment, EditorState, Prec } from '@codemirror/state'
import {
  EditorView,
  keymap,
  placeholder as placeholderExt,
  type KeyBinding,
} from '@codemirror/view'
import { surdLanguage } from './surdLang'

export interface CodeEditorHandle {
  get(): string
  set(doc: string): void
  focus(): void
}

interface Props {
  initialDoc?: string
  /** 'surd' enables highlighting + completion; 'plain' for markdown source. */
  lang?: 'surd' | 'plain'
  placeholder?: string
  autoFocus?: boolean
  editable?: boolean
  /** Bindings that run before everything else (Enter, ArrowUp, Escape…). */
  keys?: KeyBinding[]
  onDocChange?: (doc: string) => void
}

// All colors come from the theme tokens in index.css, so the editor follows
// data-mode/data-theme without rebuilding the view.
const baseTheme = EditorView.theme({
  '&': { backgroundColor: 'transparent', fontSize: '14px' },
  '.cm-content': {
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    caretColor: 'var(--ink)',
    padding: '2px 0',
  },
  '.cm-line': { padding: '0 2px 0 0' },
  '&.cm-focused': { outline: 'none' },
  '.cm-cursor': { borderLeftColor: 'var(--ink)' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 24%, transparent)',
  },
  '.cm-placeholder': { color: 'var(--faint)' },
  '.cm-tooltip': {
    backgroundColor: 'var(--raised)',
    border: '1px solid var(--edge-strong)',
    borderRadius: '6px',
  },
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li': {
    color: 'var(--ink)',
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '13px',
  },
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: 'color-mix(in srgb, var(--accent) 20%, transparent)',
    color: 'var(--ink)',
  },
  '.cm-completionDetail': { color: 'var(--faint)', fontStyle: 'normal' },
})

export const CodeEditor = forwardRef<CodeEditorHandle, Props>(function CodeEditor(
  { initialDoc = '', lang = 'surd', placeholder, autoFocus, editable = true, keys, onDocChange },
  ref,
) {
  const hostRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const editableComp = useRef(new Compartment())
  const placeholderComp = useRef(new Compartment())

  // Latest callbacks without rebuilding the view.
  const keysRef = useRef(keys)
  keysRef.current = keys
  const onDocChangeRef = useRef(onDocChange)
  onDocChangeRef.current = onDocChange

  useEffect(() => {
    // Bindings delegate through keysRef so parents can pass fresh closures
    // every render without rebuilding the view.
    const dynamicKeys: KeyBinding[] = (keysRef.current ?? []).map((k, i) => ({
      ...k,
      run: (view: EditorView) => keysRef.current?.[i]?.run?.(view) ?? false,
    }))
    const view = new EditorView({
      parent: hostRef.current!,
      state: EditorState.create({
        doc: initialDoc,
        extensions: [
          Prec.highest(keymap.of(dynamicKeys)),
          history(),
          keymap.of([...completionKeymap, ...historyKeymap, ...defaultKeymap]),
          lang === 'surd' ? surdLanguage() : [],
          placeholderComp.current.of(placeholder ? placeholderExt(placeholder) : []),
          baseTheme,
          EditorView.lineWrapping,
          editableComp.current.of(EditorView.editable.of(editable)),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onDocChangeRef.current?.(u.state.doc.toString())
          }),
        ],
      }),
    })
    viewRef.current = view
    if (autoFocus) view.focus()
    return () => {
      view.destroy()
      viewRef.current = null
    }
    // The view is intentionally built once; initialDoc/lang don't change for
    // a mounted editor (cells remount via key).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: editableComp.current.reconfigure(EditorView.editable.of(editable)),
    })
  }, [editable])

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: placeholderComp.current.reconfigure(
        placeholder ? placeholderExt(placeholder) : [],
      ),
    })
  }, [placeholder])

  useImperativeHandle(ref, () => ({
    get: () => viewRef.current?.state.doc.toString() ?? '',
    set: (doc: string) => {
      const view = viewRef.current
      if (!view) return
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: doc },
        selection: { anchor: doc.length },
      })
    },
    focus: () => viewRef.current?.focus(),
  }))

  return <div ref={hostRef} className="min-w-0 flex-1" />
})
