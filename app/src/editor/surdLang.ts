// CodeMirror language support for the surd CAS input language: a stream
// tokenizer (the syntax is line-regular enough that a full Lezer grammar
// buys nothing), a highlight style driven by the app's theme tokens, a
// completion source over builtins + keywords + live workspace names, and a
// signature tooltip that tracks the enclosing builtin call while typing.

import {
  acceptCompletion,
  autocompletion,
  snippetCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from '@codemirror/autocomplete'
import {
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
} from '@codemirror/language'
import {
  Prec,
  StateField,
  type EditorState,
  type Extension,
} from '@codemirror/state'
import { keymap, showTooltip, type Tooltip } from '@codemirror/view'
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

interface Builtin {
  name: string
  /** Display parameter names; a '?' suffix marks the argument optional
   * (shown in the signature, left out of the inserted snippet). */
  params: string[]
  doc: string
  /** Extra arguments beyond `params` are accepted (plot curves, struct
   * fields); the signature tooltip then pins the highlight to the last
   * parameter instead of dropping it. */
  variadic?: boolean
}

// Mirrors the builtin dispatch in src/eval.rs (call_builtin / call_calculus);
// `params` must stay in sync with the arity checks there.
const BUILTINS: Builtin[] = [
  {
    name: 'abs',
    params: ['x'],
    doc: 'Absolute value (modulus for complex x).',
  },
  {
    name: 'charpoly',
    params: ['M', 'var?'],
    doc: 'Characteristic polynomial of M, in var (default lambda).',
  },
  { name: 'conj', params: ['z'], doc: 'Complex conjugate.' },
  { name: 'cos', params: ['x'], doc: 'Cosine.' },
  { name: 'det', params: ['M'], doc: 'Determinant of a matrix.' },
  {
    name: 'bound',
    params: ['s', 'i?'],
    doc: 'Certified max |true − mid| of a signal (or one sample).',
  },
  {
    name: 'dot',
    params: ['a', 'b'],
    doc: 'Sum of elementwise products of two vectors.',
  },
  {
    name: 'diff',
    params: ['expr', 'x'],
    doc: 'Derivative of expr with respect to x.',
  },
  {
    name: 'eig',
    params: ['M'],
    doc: 'Eigenvalues of M (alias of eigenvalues).',
  },
  { name: 'eigenvalues', params: ['M'], doc: 'Eigenvalues of a matrix.' },
  { name: 'eigenvectors', params: ['M'], doc: 'Eigenvectors of a matrix.' },
  { name: 'exp', params: ['x'], doc: 'Exponential function.' },
  {
    name: 'expand',
    params: ['expr'],
    doc: 'Expand products and integer powers.',
  },
  {
    name: 'eye',
    params: ['n'],
    doc: 'n×n identity matrix (alias of identity).',
  },
  { name: 'identity', params: ['n'], doc: 'n×n identity matrix.' },
  { name: 'im', params: ['z'], doc: 'Imaginary part (alias of imag).' },
  { name: 'imag', params: ['z'], doc: 'Imaginary part.' },
  { name: 'inv', params: ['M'], doc: 'Matrix inverse.' },
  {
    name: 'hcat',
    params: ['a', 'b'],
    doc: 'Stack matrices horizontally; accepts any number of pieces.',
    variadic: true,
  },
  {
    name: 'kernel',
    params: ['M'],
    doc: 'Nullspace basis of M (alias of nullspace).',
  },
  {
    name: 'len',
    params: ['v'],
    doc: 'Entries of a vector; rows of a matrix.',
  },
  {
    name: 'linspace',
    params: ['a', 'b', 'n'],
    doc: 'n evenly spaced points from a to b, exact step.',
  },
  { name: 'ln', params: ['x'], doc: 'Natural logarithm.' },
  { name: 'lu', params: ['M'], doc: 'LU decomposition.' },
  {
    name: 'map',
    params: ['f', 'm'],
    doc: 'Apply a function entrywise, preserving shape.',
  },
  {
    name: 'mid',
    params: ['s'],
    doc: 'Midpoints of a signal, as a column matrix.',
  },
  {
    name: 'N',
    params: ['x', 'digits?'],
    doc: 'Numeric value of x, to digits (default set by precision).',
  },
  { name: 'nullspace', params: ['M'], doc: 'Nullspace basis of a matrix.' },
  {
    name: 'plot',
    params: ['f', 'x', 'a', 'b'],
    doc: 'Plot f over x in [a, b] (overlay: more functions before x) — or plot(s) for signals.',
    variadic: true,
  },
  {
    name: 'plot3d',
    params: ['f', 'x', 'a', 'b', 'y', 'c', 'd'],
    doc: 'Surface z = f(x, y) over [a, b] × [c, d].',
  },
  {
    name: 'precision',
    params: ['digits?'],
    doc: 'Query, or set, the default numeric precision.',
  },
  { name: 'qr', params: ['M'], doc: 'QR decomposition.' },
  { name: 'rank', params: ['M'], doc: 'Rank of a matrix.' },
  { name: 're', params: ['z'], doc: 'Real part (alias of real).' },
  { name: 'real', params: ['z'], doc: 'Real part.' },
  { name: 'rref', params: ['M'], doc: 'Reduced row echelon form.' },
  { name: 'sin', params: ['x'], doc: 'Sine.' },
  {
    name: 'signal',
    params: ['v', 'digits?'],
    doc: 'Pack a vector as certified bulk data (f64, or arbitrary precision).',
  },
  {
    name: 'size',
    params: ['m'],
    doc: 'Dimensions of a matrix → struct(rows, cols).',
  },
  {
    name: 'slice',
    params: ['v', 'start', 'n'],
    doc: 'n consecutive elements from 1-based start (vectors and signals).',
  },
  {
    name: 'solve',
    params: ['A', 'b'],
    doc: 'Solve the linear system A·x = b.',
  },
  { name: 'sqrt', params: ['x'], doc: 'Square root.' },
  {
    name: 'struct',
    params: ['name = value'],
    doc: 'Build a struct; pass one name = value per field.',
    variadic: true,
  },
  {
    name: 'subs',
    params: ['expr', 'x', 'val'],
    doc: 'Substitute val for x in expr.',
  },
  { name: 'tan', params: ['x'], doc: 'Tangent.' },
  { name: 'transpose', params: ['M'], doc: 'Matrix transpose.' },
  {
    name: 'vcat',
    params: ['a', 'b'],
    doc: 'Stack matrices vertically; accepts any number of pieces.',
    variadic: true,
  },
]

/** A built-in namespace (see is_namespace / call_namespace in src/eval.rs);
 * members mirror the dispatch in the namespace's module (src/dsp.rs,
 * src/stats.rs). */
interface Namespace {
  name: string
  doc: string
  members: Builtin[]
}

const NAMESPACES: Namespace[] = [
  {
    name: 'dsp',
    doc: 'Exact digital signal processing.',
    members: [
      {
        name: 'circconv',
        params: ['a', 'b'],
        doc: 'Circular convolution of two equal-length vectors.',
      },
      {
        name: 'blackman',
        params: ['n'],
        doc: 'Blackman window, exact rational coefficients.',
      },
      {
        name: 'conv',
        params: ['a', 'b'],
        doc: 'Linear convolution, length m+n−1.',
      },
      { name: 'dft', params: ['v'], doc: 'Discrete Fourier transform, exact.' },
      { name: 'dftmatrix', params: ['n'], doc: 'The n×n Fourier matrix.' },
      {
        name: 'fft',
        params: ['s'],
        doc: 'Certified radix-2 FFT of a signal → struct(re, im).',
      },
      {
        name: 'firlow',
        params: ['n', 'wc'],
        doc: 'Windowed-sinc lowpass prototype, cutoff wc rad/sample.',
      },
      {
        name: 'freqz',
        params: ['h', 'w'],
        doc: 'FIR frequency response at the frequencies in w.',
      },
      { name: 'hamming', params: ['n'], doc: 'Hamming window (27/50, 23/50).' },
      {
        name: 'remez',
        params: ['n', 'edges', 'desired', 'weights?'],
        doc: 'Exact Parks–McClellan equiripple FIR design → struct(taps, ripple).',
      },
      { name: 'hann', params: ['n'], doc: 'Hann window.' },
      {
        name: 'idft',
        params: ['v'],
        doc: 'Inverse DFT; exactly inverts dsp.dft.',
      },
      {
        name: 'ifft',
        params: ['f'],
        doc: 'Certified inverse FFT of struct(re, im).',
      },
      {
        name: 'pad',
        params: ['s', 'n'],
        doc: 'Zero-pad a signal to length n (never truncates).',
      },
      {
        name: 'peak',
        params: ['s'],
        doc: 'Certified upper bound on max |x| of a signal.',
      },
      {
        name: 'quantize',
        params: ['v', 'bits'],
        doc: 'Snap to a fixed-point grid (bits fractional bits), exactly.',
      },
      {
        name: 'rms',
        params: ['s'],
        doc: 'Certified upper bound on the RMS of a signal.',
      },
      {
        name: 'window',
        params: ['name', 'n'],
        doc: 'Certified window signal (hann, hamming, blackman) for bulk data.',
      },
    ],
  },
  {
    name: 'stats',
    doc: 'Exact statistics.',
    members: [
      {
        name: 'cor',
        params: ['a', 'b'],
        doc: 'Pearson correlation; exactly ±1 for linear data.',
      },
      { name: 'cov', params: ['a', 'b'], doc: 'Sample covariance.' },
      {
        name: 'linfit',
        params: ['x', 'y'],
        doc: 'Exact least-squares line → struct(intercept, slope).',
      },
      {
        name: 'lsq',
        params: ['A', 'b'],
        doc: 'General exact least squares: β minimizing ‖Aβ − b‖.',
      },
      { name: 'mean', params: ['v'], doc: 'Mean, exact.' },
      { name: 'median', params: ['v'], doc: 'Median by exact ordering.' },
      {
        name: 'polyfit',
        params: ['x', 'y', 'deg'],
        doc: 'Exact least-squares polynomial; coefficients, constant first.',
      },
      {
        name: 'polyval',
        params: ['c', 't'],
        doc: 'Evaluate a coefficient vector at t (scalar, symbol, or vector).',
      },
      {
        name: 'quantile',
        params: ['v', 'q'],
        doc: 'q-th quantile by exact interpolation (0 ≤ q ≤ 1).',
      },
      {
        name: 'r2',
        params: ['y', 'yhat'],
        doc: 'Coefficient of determination; exactly 1 for a perfect fit.',
      },
      {
        name: 'rmse',
        params: ['a', 'b'],
        doc: 'Root mean squared error, as an exact surd.',
      },
      {
        name: 'std',
        params: ['v'],
        doc: 'Sample standard deviation (an exact surd).',
      },
      { name: 'var', params: ['v'], doc: 'Sample variance (n−1 denominator).' },
    ],
  },
]

const KEYWORD_SET = new Set(KEYWORDS)
const CONSTANT_SET = new Set(CONSTANTS)
const BUILTIN_BY_NAME = new Map(BUILTINS.map((b) => [b.name, b]))
const NAMESPACE_BY_NAME = new Map(NAMESPACES.map((n) => [n.name, n]))
// Qualified names ('dsp.dft') join the lookup for signature help; a bare
// identifier token can never contain a dot, so highlighting is unaffected.
for (const ns of NAMESPACES) {
  for (const m of ns.members) {
    BUILTIN_BY_NAME.set(`${ns.name}.${m.name}`, {
      ...m,
      name: `${ns.name}.${m.name}`,
    })
  }
}

function signature(b: Builtin): string {
  return '(' + b.params.join(', ') + ')'
}

/** Snippet template over the required parameters: `diff(${expr}, ${x})`.
 * Optional parameters are omitted; an all-optional list leaves a single
 * empty field inside the parens so Tab still lands there. */
function snippetTemplate(b: Builtin): string {
  const fields = b.params
    .filter((p) => !p.endsWith('?'))
    .map((p) => '${' + p + '}')
  return `${b.name}(${fields.length ? fields.join(', ') : '${}'})`
}

const surdStream = StreamLanguage.define<void>({
  token(stream) {
    if (stream.eatSpace()) return null
    if (stream.match(/^(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?/)) return 'number'
    if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*/)) {
      const w = stream.current()
      if (KEYWORD_SET.has(w)) return 'keyword'
      if (CONSTANT_SET.has(w)) return 'atom'
      if (BUILTIN_BY_NAME.has(w)) return 'builtin'
      // Namespaces highlight as builtins when used as one (`dsp.`), and so
      // do their members right after the dot (`dsp.dft`); a bare `dsp` is
      // an ordinary variable, matching the engine's shadowing rule.
      if (NAMESPACE_BY_NAME.has(w) && stream.peek() === '.') return 'builtin'
      const qualifier = /([A-Za-z_][A-Za-z0-9_]*)\.$/.exec(
        stream.string.slice(0, stream.start),
      )
      if (
        qualifier &&
        NAMESPACE_BY_NAME.get(qualifier[1])?.members.some((m) => m.name === w)
      ) {
        return 'builtin'
      }
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
  ...BUILTINS.map((b) =>
    snippetCompletion(snippetTemplate(b), {
      label: b.name,
      type: 'function',
      detail: signature(b),
      info: `${b.name}${signature(b)} — ${b.doc}`,
    }),
  ),
  ...NAMESPACES.map((n) => ({
    label: n.name,
    type: 'namespace',
    detail: 'namespace',
    info: `${n.name} — ${n.doc}`,
  })),
  ...KEYWORDS.map((k) => ({ label: k, type: 'keyword' })),
  ...CONSTANTS.map((c) => ({ label: c, type: 'constant' })),
]

/** Builtins + keywords + whatever is bound in the live workspace. */
function completionSource(context: CompletionContext): CompletionResult | null {
  // Right of a namespace dot, complete its members (and only those).
  const member = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z0-9_]*/)
  if (member) {
    const dot = member.text.indexOf('.')
    const ns = NAMESPACE_BY_NAME.get(member.text.slice(0, dot))
    if (ns) {
      return {
        from: member.from + dot + 1,
        options: ns.members.map((m) =>
          snippetCompletion(snippetTemplate(m), {
            label: m.name,
            type: 'function',
            detail: signature(m),
            info: `${ns.name}.${m.name}${signature(m)} — ${m.doc}`,
          }),
        ),
        validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
      }
    }
  }
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

// --- Signature help -------------------------------------------------------
// While the cursor sits inside the arguments of a builtin call, a tooltip
// above the call shows its signature with the argument being typed
// highlighted (the docs line from the completion repeats below it).

/** How far back to scan for the enclosing call; cells are small, this only
 * guards against pathological documents. */
const SCAN_LIMIT = 1000

interface CallSite {
  builtin: Builtin
  /** 0-based index of the argument the cursor is in. */
  argIndex: number
  /** Document position of the call's opening paren. */
  pos: number
}

/** Innermost enclosing builtin call at the cursor. Scans backwards keeping a
 * bracket depth; commas at depth 0 count arguments. An unmatched opener that
 * is not a builtin call — a grouping paren, a matrix bracket, an unknown
 * function — is transparent: scanning continues outside it, and the commas
 * collected so far are discarded as its own. */
function enclosingCall(state: EditorState): CallSite | null {
  const head = state.selection.main.head
  const from = Math.max(0, head - SCAN_LIMIT)
  const text = state.doc.sliceString(from, head)
  let depth = 0
  let commas = 0
  for (let i = text.length - 1; i >= 0; i--) {
    const ch = text[i]
    if (ch === ')' || ch === ']' || ch === '}') depth++
    else if (ch === ',' && depth === 0) commas++
    else if (ch === '(' || ch === '[' || ch === '{') {
      if (depth > 0) {
        depth--
        continue
      }
      if (ch === '(') {
        let start = i
        // Dots included, so a qualified call (`dsp.dft(`) resolves whole.
        while (start > 0 && /[A-Za-z0-9_.]/.test(text[start - 1])) start--
        const builtin = BUILTIN_BY_NAME.get(text.slice(start, i))
        if (builtin) return { builtin, argIndex: commas, pos: from + i }
      }
      commas = 0
    }
  }
  return null
}

function signatureTooltip(state: EditorState): Tooltip | null {
  const call = enclosingCall(state)
  if (!call) return null
  const { builtin, argIndex } = call
  const active =
    argIndex < builtin.params.length
      ? argIndex
      : builtin.variadic
        ? builtin.params.length - 1
        : -1
  return {
    pos: call.pos,
    above: true,
    create: () => {
      const dom = document.createElement('div')
      dom.className = 'cm-surd-signature'
      const line = dom.appendChild(document.createElement('div'))
      line.appendChild(document.createTextNode(builtin.name + '('))
      builtin.params.forEach((p, i) => {
        if (i > 0) line.appendChild(document.createTextNode(', '))
        const span = line.appendChild(document.createElement('span'))
        if (i === active) span.className = 'cm-surd-signature-active'
        span.textContent = p
      })
      line.appendChild(document.createTextNode(')'))
      const doc = dom.appendChild(document.createElement('div'))
      doc.className = 'cm-surd-signature-doc'
      doc.textContent = builtin.doc
      return { dom }
    },
  }
}

// The field tracks the cursor unconditionally; the tooltip is hidden by CSS
// while the editor is blurred (selections persist in blurred cells, and a
// notebook full of stale signature tooltips would be noise). Focus is handled
// in CSS rather than state because the focusChangeEffect transaction is
// silently dropped when another transaction races it at mount.
const signatureField = StateField.define<Tooltip | null>({
  create: signatureTooltip,
  update(value, tr) {
    if (!tr.docChanged && !tr.selection) return value
    return signatureTooltip(tr.state)
  },
  provide: (f) => showTooltip.from(f),
})

export function surdLanguage(): Extension {
  return [
    surdStream,
    syntaxHighlighting(highlight),
    autocompletion({ override: [completionSource], icons: false }),
    // Tab accepts the selected completion. Registered before the snippet
    // keymap (which appends itself to the configuration when a snippet
    // activates), so with the popup open Tab completes; otherwise it falls
    // through to snippet-field navigation.
    Prec.highest(keymap.of([{ key: 'Tab', run: acceptCompletion }])),
    signatureField,
  ]
}
