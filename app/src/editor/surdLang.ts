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
import {
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
} from '@codemirror/language'
import type { Extension } from '@codemirror/state'
import { tags as t } from '@lezer/highlight'
import { useNotebook } from '../state/store'

const KEYWORDS = [
  'if',
  'then',
  'else',
  'end',
  'while',
  'do',
  'function',
  'and',
  'or',
  'not',
]
const CONSTANTS = ['pi', 'e', 'true', 'false']
// Mirrors the builtin dispatch in src/eval.rs (call_builtin / call_calculus).
// `sig` is the parameter list shown next to the name in the completion list
// and must stay in sync with the arity checks there; a trailing `?` marks an
// optional argument.
const BUILTINS: { name: string; sig: string; doc: string }[] = [
  { name: 'abs', sig: '(x)', doc: 'Absolute value (modulus for complex x).' },
  {
    name: 'charpoly',
    sig: '(M, var?)',
    doc: 'Characteristic polynomial of M, in var (default lambda).',
  },
  { name: 'conj', sig: '(z)', doc: 'Complex conjugate.' },
  { name: 'cos', sig: '(x)', doc: 'Cosine.' },
  { name: 'det', sig: '(M)', doc: 'Determinant of a matrix.' },
  {
    name: 'diff',
    sig: '(expr, x)',
    doc: 'Derivative of expr with respect to x.',
  },
  { name: 'eig', sig: '(M)', doc: 'Eigenvalues of M (alias of eigenvalues).' },
  { name: 'eigenvalues', sig: '(M)', doc: 'Eigenvalues of a matrix.' },
  { name: 'eigenvectors', sig: '(M)', doc: 'Eigenvectors of a matrix.' },
  { name: 'exp', sig: '(x)', doc: 'Exponential function.' },
  { name: 'expand', sig: '(expr)', doc: 'Expand products and integer powers.' },
  { name: 'eye', sig: '(n)', doc: 'n×n identity matrix (alias of identity).' },
  { name: 'identity', sig: '(n)', doc: 'n×n identity matrix.' },
  { name: 'im', sig: '(z)', doc: 'Imaginary part (alias of imag).' },
  { name: 'imag', sig: '(z)', doc: 'Imaginary part.' },
  { name: 'inv', sig: '(M)', doc: 'Matrix inverse.' },
  {
    name: 'kernel',
    sig: '(M)',
    doc: 'Nullspace basis of M (alias of nullspace).',
  },
  { name: 'ln', sig: '(x)', doc: 'Natural logarithm.' },
  { name: 'lu', sig: '(M)', doc: 'LU decomposition.' },
  {
    name: 'N',
    sig: '(x, digits?)',
    doc: 'Numeric value of x, to digits (default set by precision).',
  },
  { name: 'nullspace', sig: '(M)', doc: 'Nullspace basis of a matrix.' },
  {
    name: 'plot',
    sig: '(f1, ..., fk, x, a, b)',
    doc: 'Plot one or more curves in x over [a, b].',
  },
  {
    name: 'plot3d',
    sig: '(f, x, a, b, y, c, d)',
    doc: 'Surface z = f(x, y) over [a, b] × [c, d].',
  },
  {
    name: 'precision',
    sig: '(digits?)',
    doc: 'Query, or set, the default numeric precision.',
  },
  { name: 'qr', sig: '(M)', doc: 'QR decomposition.' },
  { name: 'rank', sig: '(M)', doc: 'Rank of a matrix.' },
  { name: 're', sig: '(z)', doc: 'Real part (alias of real).' },
  { name: 'real', sig: '(z)', doc: 'Real part.' },
  { name: 'rref', sig: '(M)', doc: 'Reduced row echelon form.' },
  { name: 'sin', sig: '(x)', doc: 'Sine.' },
  { name: 'solve', sig: '(A, b)', doc: 'Solve the linear system A·x = b.' },
  { name: 'sqrt', sig: '(x)', doc: 'Square root.' },
  {
    name: 'struct',
    sig: '(name = value, ...)',
    doc: 'Build a struct from name = value fields.',
  },
  { name: 'subs', sig: '(expr, x, val)', doc: 'Substitute val for x in expr.' },
  { name: 'tan', sig: '(x)', doc: 'Tangent.' },
  { name: 'transpose', sig: '(M)', doc: 'Matrix transpose.' },
]

const KEYWORD_SET = new Set(KEYWORDS)
const CONSTANT_SET = new Set(CONSTANTS)
const BUILTIN_SET = new Set(BUILTINS.map((b) => b.name))

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
  ...BUILTINS.map((b) => ({
    label: b.name,
    type: 'function',
    detail: b.sig,
    info: `${b.name}${b.sig} — ${b.doc}`,
  })),
  ...KEYWORDS.map((k) => ({ label: k, type: 'keyword' })),
  ...CONSTANTS.map((c) => ({ label: c, type: 'constant' })),
]

/** Builtins + keywords + whatever is bound in the live workspace. */
function completionSource(context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*/)
  if (!word || (word.from === word.to && !context.explicit)) return null
  const workspace = useNotebook.getState().workspace.map((entry) => ({
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
