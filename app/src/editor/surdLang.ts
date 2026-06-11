// CodeMirror language support for the surd CAS input language: a stream
// tokenizer (the syntax is line-regular enough that a full Lezer grammar
// buys nothing), a highlight style driven by the app's theme tokens, and a
// completion source over builtins + keywords + live workspace names.

import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from '@codemirror/autocomplete'
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from '@codemirror/language'
import type { Extension } from '@codemirror/state'
import { tags as t } from '@lezer/highlight'
import { useNotebook } from '../state/store'

const KEYWORDS = ['if', 'then', 'else', 'end', 'while', 'do', 'function', 'and', 'or', 'not']
const CONSTANTS = ['pi', 'e', 'true', 'false']
// Mirrors the builtin dispatch in src/eval.rs (call_builtin / call_calculus).
const BUILTINS = [
  'abs', 'charpoly', 'conj', 'cos', 'det', 'diff', 'eig', 'eigenvalues',
  'exp', 'expand', 'eye', 'identity', 'im', 'imag', 'inv', 'ln', 'N',
  'plot', 'plot3d', 'precision', 'rank', 're', 'real', 'rref', 'sin', 'solve',
  'sqrt', 'struct', 'subs', 'tan', 'transpose',
]

const KEYWORD_SET = new Set(KEYWORDS)
const CONSTANT_SET = new Set(CONSTANTS)
const BUILTIN_SET = new Set(BUILTINS)

const surdStream = StreamLanguage.define<void>({
  token(stream) {
    if (stream.eatSpace()) return null
    if (stream.match(/^(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?/)) return 'number'
    if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*/)) {
      const w = stream.current()
      if (KEYWORD_SET.has(w)) return 'keyword'
      if (CONSTANT_SET.has(w)) return 'atom'
      if (BUILTIN_SET.has(w)) return 'builtin'
      return 'variableName'
    }
    if (stream.match(/^(:=|==|!=|<=|>=|[+\-*/^=<>.])/)) return 'operator'
    if (stream.match(/^[[\](){},;]/)) return 'bracket'
    stream.next()
    return null
  },
  tokenTable: {
    number: t.number,
    keyword: t.keyword,
    atom: t.atom,
    builtin: t.function(t.variableName),
    variableName: t.variableName,
    operator: t.operator,
    bracket: t.bracket,
  },
})

// Colors come from the theme tokens in index.css (--syn-* vary by mode and,
// where the accent would clash, by theme; builtins track the accent itself).
const highlight = HighlightStyle.define([
  { tag: t.number, color: 'var(--syn-number)' },
  { tag: t.keyword, color: 'var(--syn-keyword)' },
  { tag: t.atom, color: 'var(--syn-atom)' },
  { tag: t.function(t.variableName), color: 'var(--accent)' },
  { tag: t.variableName, color: 'var(--ink)' },
  { tag: t.operator, color: 'var(--muted)' },
  { tag: t.bracket, color: 'var(--faint)' },
])

const STATIC_COMPLETIONS: Completion[] = [
  ...BUILTINS.map((b) => ({ label: b, type: 'function' })),
  ...KEYWORDS.map((k) => ({ label: k, type: 'keyword' })),
  ...CONSTANTS.map((c) => ({ label: c, type: 'constant' })),
]

/** Builtins + keywords + whatever is bound in the live workspace. */
function completionSource(context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*/)
  if (!word || (word.from === word.to && !context.explicit)) return null
  const workspace = useNotebook
    .getState()
    .workspace.map((entry) => ({
      label: entry.name,
      type: entry.kind === 'function' ? 'function' : 'variable',
      detail: entry.text.length > 24 ? entry.text.slice(0, 24) + '…' : entry.text,
    }))
  return {
    from: word.from,
    options: [...workspace, ...STATIC_COMPLETIONS],
    validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
  }
}

export function surdLanguage(): Extension {
  return [
    surdStream,
    syntaxHighlighting(highlight),
    autocompletion({ override: [completionSource], icons: false }),
  ]
}
