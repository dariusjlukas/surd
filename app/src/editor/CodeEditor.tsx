// Thin React wrapper around a CodeMirror EditorView. The view is created
// once; parents that need to read/set/clear the document do it through the
// imperative handle (the input bar's submit-and-clear and history recall
// don't fit a controlled-component shape).

import { forwardRef, useEffect, useImperativeHandle, useRef } from 'react'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { closeCompletion, completionKeymap } from '@codemirror/autocomplete'
import { Compartment, EditorState, Prec } from '@codemirror/state'
import {
  drawSelection,
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
  /** Fires when the editor loses focus (e.g. clicking away from a cell). */
  onBlur?: () => void
}

// WKWebView (the macOS Tauri webview) paints the *native* caret on its own
// blink timer, so it only repositions the caret on the next blink tick — typing
// or backspacing quickly leaves the caret lagging a character behind the real
// cursor. (It also sometimes leaves a "ghost" of the old caret behind.) So we
// stop relying on the native caret and let CodeMirror draw its own: drawSelection
// re-measures and repositions a DOM caret synchronously on every transaction, and
// hides the native one. The blink is a CSS opacity animation, independent of
// position. The one WebKit quirk drawSelection hits is an empty line, where
// coordsAtPos reports a zero-height rect and the caret div would get height:0px
// (invisible) — the .cm-cursor min-height below backstops that.

// All colors come from the theme tokens in index.css, so the editor follows
// data-mode/data-theme without rebuilding the view.
const baseTheme = EditorView.theme({
  '&': { backgroundColor: 'transparent', fontSize: '14px' },
  '.cm-content': {
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    padding: '2px 0',
  },
  '.cm-line': { padding: '0 2px 0 0' },
  '&.cm-focused': { outline: 'none' },
  // borderLeftColor colors the drawn caret; min-height keeps it visible on
  // empty lines where WebKit measures a zero-height cursor rect (see above).
  '.cm-cursor': { borderLeftColor: 'var(--ink)', minHeight: '1.2em' },
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
  '.cm-tooltip.cm-completionInfo': {
    color: 'var(--muted)',
    fontSize: '12px',
    maxWidth: '320px',
    padding: '4px 8px',
  },
  '&:not(.cm-focused) .cm-tooltip.cm-surd-signature': { display: 'none' },
  '.cm-tooltip.cm-surd-signature': {
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '12px',
    color: 'var(--muted)',
    padding: '4px 8px',
  },
  '.cm-surd-signature-active': {
    color: 'var(--accent)',
    fontWeight: '600',
  },
  '.cm-surd-signature-doc': {
    color: 'var(--faint)',
    fontSize: '11px',
    marginTop: '2px',
  },
})

export const CodeEditor = forwardRef<CodeEditorHandle, Props>(
  function CodeEditor(
    {
      initialDoc = '',
      lang = 'surd',
      placeholder,
      autoFocus,
      editable = true,
      keys,
      onDocChange,
      onBlur,
    },
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
    const onBlurRef = useRef(onBlur)
    onBlurRef.current = onBlur

    useEffect(() => {
      // True only while this view is the live, mounted one. Destroying a
      // focused view fires a `blur` synchronously — and in dev StrictMode the
      // first mount is torn down immediately — so the handler must ignore
      // blurs raised during teardown, or it would collapse a just-opened cell.
      let alive = false
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
            // With the completion popup open, Escape dismisses it; only a
            // second Escape reaches the parent bindings below (clear input /
            // cancel cell edit). Must precede dynamicKeys to win the tie.
            Prec.highest(keymap.of([{ key: 'Escape', run: closeCompletion }])),
            Prec.highest(keymap.of(dynamicKeys)),
            history(),
            keymap.of([
              ...completionKeymap,
              ...historyKeymap,
              ...defaultKeymap,
            ]),
            lang === 'surd' ? surdLanguage() : [],
            placeholderComp.current.of(
              placeholder ? placeholderExt(placeholder) : [],
            ),
            baseTheme,
            // CM draws and positions its own caret synchronously per transaction
            // and hides the native one, so the caret can't lag behind on WebKit.
            drawSelection(),
            EditorView.lineWrapping,
            editableComp.current.of(EditorView.editable.of(editable)),
            EditorView.updateListener.of((u) => {
              if (u.docChanged) onDocChangeRef.current?.(u.state.doc.toString())
            }),
            EditorView.domEventHandlers({
              blur: () => {
                if (alive) onBlurRef.current?.()
              },
            }),
          ],
        }),
      })
      viewRef.current = view
      alive = true
      if (autoFocus) view.focus()
      return () => {
        alive = false
        view.destroy()
        viewRef.current = null
      }
      // The view is intentionally built once; initialDoc/lang don't change for
      // a mounted editor (cells remount via key).
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    useEffect(() => {
      viewRef.current?.dispatch({
        effects: editableComp.current.reconfigure(
          EditorView.editable.of(editable),
        ),
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
  },
)
