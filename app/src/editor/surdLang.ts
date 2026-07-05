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
  { name: 'beta', params: ['a', 'b'], doc: 'Beta function B(a, b).' },
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
  { name: 'erf', params: ['x'], doc: 'Error function.' },
  {
    name: 'erfc',
    params: ['x'],
    doc: 'Complementary error function, 1 − erf(x).',
  },
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
  {
    name: 'fill',
    params: ['value', 'rows', 'cols?'],
    doc: 'Matrix of a constant value: fill(v, n) is n×n, fill(v, rows, cols) is rows×cols. If value is a function, each entry is f(row, col).',
  },
  { name: 'identity', params: ['n'], doc: 'n×n identity matrix.' },
  { name: 'im', params: ['z'], doc: 'Imaginary part (alias of imag).' },
  { name: 'imag', params: ['z'], doc: 'Imaginary part.' },
  { name: 'inv', params: ['M'], doc: 'Matrix inverse.' },
  {
    name: 'gamma',
    params: ['x'],
    doc: 'Gamma function (factorial on positive integers).',
  },
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
  { name: 'lgamma', params: ['x'], doc: 'Log-gamma, ln Γ(x), for x > 0.' },
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
    doc: 'Plot f over x in [a, b] (overlay more functions, or scatter(x, y) data, before x) — or plot(s) for signals.',
    variadic: true,
  },
  {
    name: 'plot3d',
    params: ['f', 'x', 'a', 'b', 'y', 'c', 'd'],
    doc: 'Surface z = f(x, y) over [a, b] × [c, d] (overlay scatter3d(x, y, z) data before x) — or plot3d(scatter3d(…)) alone.',
    variadic: true,
  },
  {
    name: 'scatter',
    params: ['x', 'y'],
    doc: 'Data points (x, y) as markers; overlay in plot(…) to compare with a curve, or plot(scatter(x, y)) alone.',
  },
  {
    name: 'scatter3d',
    params: ['x', 'y', 'z'],
    doc: '3D data points (x, y, z) as markers; overlay in plot3d(…) over a surface, or plot3d(scatter3d(x, y, z)) alone.',
  },
  {
    name: 'spectrogram',
    params: ['s', 'nfft?', 'hop?'],
    doc: 'STFT heatmap of a signal (Hann window, dB): time × frequency. nfft a power of two; hop defaults to nfft/4.',
  },
  {
    name: 'pairs',
    params: ['M'],
    doc: 'Scatterplot matrix of a data matrix (columns are variables) or a struct of columns — pairs(M, [name1, …]) labels the columns.',
    variadic: true,
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
  {
    name: 'root',
    params: ['p', 'k'],
    doc: 'The k-th real root (ascending, 1-based) of a polynomial — exact, even without a radical form: root(x^5 - x - 1, 1).',
  },
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
 * src/stats.rs, src/data.rs). */
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
        name: 'butter',
        params: ['n', 'wc', 'kind?'],
        doc: 'Exact Butterworth design (bilinear, prewarped) → struct(sos, order, kind); kind is lowpass (default) or highpass.',
      },
      {
        name: 'filter',
        params: ['b', 'a', 'x'],
        doc: 'Exact recursive (IIR) filtering of a vector; also filter(f, x) with a filter struct (SOS cascade).',
      },
      {
        name: 'freqz',
        params: ['h', 'w'],
        doc: 'Frequency response at the frequencies in w: FIR taps, freqz(b, a, w), or freqz(f, w) with a filter struct.',
      },
      {
        name: 'impz',
        params: ['f', 'n'],
        doc: 'First n samples of the impulse response, exactly; also impz(b, a, n).',
      },
      {
        name: 'stft',
        params: ['v', 'nfft', 'hop'],
        doc: 'Exact short-time Fourier transform of a vector (periodic Hann) → struct(frames, nfft, hop).',
      },
      {
        name: 'stable',
        params: ['f'],
        doc: 'Certified strict stability (all poles inside the unit circle) — exact Schur–Cohn; takes a filter, SOS matrix, or denominator coefficients.',
      },
      { name: 'hamming', params: ['n'], doc: 'Hamming window (27/50, 23/50).' },
      {
        name: 'remez',
        params: ['n', 'edges', 'desired', 'weights?', 'antisymmetric?'],
        doc: 'Exact Parks–McClellan equiripple FIR design, all four linear-phase types (even n → Type II; trailing antisymmetric → III/IV, e.g. Hilbert) → struct(taps, ripple, fir_type).',
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
        doc: 'Exact least-squares line → struct(intercept, slope, predict).',
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
      // Regression models, inference, and diagnostics. A multi-predictor X is a
      // design matrix; these models also accept a `y ~ x1 + x2` formula plus a
      // data struct in place of (X, y).
      {
        name: 'regress',
        params: ['X', 'y'],
        doc: 'Ordinary least squares with full inference → fitted-model struct.',
      },
      {
        name: 'wls',
        params: ['X', 'y', 'weights'],
        doc: 'Weighted least squares (per-observation weights).',
      },
      {
        name: 'ridge',
        params: ['X', 'y', 'lambda'],
        doc: 'L2-penalized (ridge) regression; exact in lambda.',
      },
      {
        name: 'lasso',
        params: ['X', 'y', 'lambda'],
        doc: 'L1-penalized (lasso) regression; zeroes out coefficients (selection).',
      },
      {
        name: 'cv',
        params: ['X|formula', 'y|data', 'k', 'opts?'],
        doc: 'k-fold cross-validation (seeded, reproducible); opts: struct(model, lambda, seed).',
      },
      {
        name: 'logit',
        params: ['X', 'y'],
        doc: 'Logistic regression by IRLS, for a binary 0/1 response y.',
      },
      {
        name: 'nlfit',
        params: ['model', 'params', 'x', 'y', 'init?'],
        doc: 'Nonlinear least squares; params is a [list] of names, Jacobian exact.',
      },
      {
        name: 'predict',
        params: ['model', 'Xnew', 'level?'],
        doc: 'Predict from a model, with confidence and prediction intervals.',
      },
      {
        name: 'robustse',
        params: ['model', 'X', 'type?'],
        doc: 'Heteroskedasticity-consistent (HC0–HC3) standard errors.',
      },
      {
        name: 'anova',
        params: ['reduced', 'full'],
        doc: 'F-test comparing two nested regression models.',
      },
      {
        name: 'dwtest',
        params: ['model'],
        doc: 'Durbin–Watson test for residual autocorrelation.',
      },
      {
        name: 'bptest',
        params: ['model'],
        doc: 'Breusch–Pagan test for heteroskedasticity.',
      },
      {
        name: 'jbtest',
        params: ['model'],
        doc: 'Jarque–Bera test for non-normal residuals.',
      },
      {
        name: 'ttest',
        params: ['x', 'mu|y', 'paired?'],
        doc: 't-test: one-sample (x, mu), two-sample Welch (x, y), or (x, y, paired).',
      },
      {
        name: 'chisqtest',
        params: ['table|x', 'y?'],
        doc: 'Chi-square independence test on a contingency table or two categorical columns.',
      },
      {
        name: 'cortest',
        params: ['x', 'y'],
        doc: 'Test whether the Pearson correlation is zero (exact r, symbolic p).',
      },
      // Probability distributions: each symbolic until N(...). CDF, PDF, inverse.
      {
        name: 'normcdf',
        params: ['x', 'mu?', 'sigma?'],
        doc: 'Normal CDF (default mean 0, std 1).',
      },
      {
        name: 'normpdf',
        params: ['x', 'mu?', 'sigma?'],
        doc: 'Normal PDF (default mean 0, std 1).',
      },
      {
        name: 'norminv',
        params: ['p', 'mu?', 'sigma?'],
        doc: 'Normal inverse CDF / quantile (default mean 0, std 1).',
      },
      {
        name: 'tcdf',
        params: ['t', 'nu'],
        doc: 'Student-t CDF with nu degrees of freedom.',
      },
      {
        name: 'tpdf',
        params: ['t', 'nu'],
        doc: 'Student-t PDF with nu degrees of freedom.',
      },
      {
        name: 'tinv',
        params: ['p', 'nu'],
        doc: 'Student-t inverse CDF / quantile.',
      },
      {
        name: 'chisqcdf',
        params: ['x', 'k'],
        doc: 'Chi-square CDF with k degrees of freedom.',
      },
      {
        name: 'chisqpdf',
        params: ['x', 'k'],
        doc: 'Chi-square PDF with k degrees of freedom.',
      },
      {
        name: 'chisqinv',
        params: ['p', 'k'],
        doc: 'Chi-square inverse CDF / quantile.',
      },
      {
        name: 'fcdf',
        params: ['x', 'd1', 'd2'],
        doc: 'F CDF with (d1, d2) degrees of freedom.',
      },
      {
        name: 'fpdf',
        params: ['x', 'd1', 'd2'],
        doc: 'F PDF with (d1, d2) degrees of freedom.',
      },
      {
        name: 'finv',
        params: ['p', 'd1', 'd2'],
        doc: 'F inverse CDF / quantile.',
      },
    ],
  },
  {
    name: 'data',
    doc: 'Data preparation for the stats models.',
    members: [
      {
        name: 'center',
        params: ['v'],
        doc: 'Subtract the mean (mean-centered column).',
      },
      {
        name: 'dropna',
        params: ['v'],
        doc: 'Drop rows with missing values (NA) from a vector, matrix, or table.',
      },
      {
        name: 'dummy',
        params: ['v'],
        doc: 'One-hot encode a categorical column → struct(levels, indicators).',
      },
      {
        name: 'groupby',
        params: ['keys', 'values'],
        doc: 'Aggregate values by levels of keys → struct(levels, count, sum, mean).',
      },
      {
        name: 'rescale',
        params: ['v'],
        doc: 'Min–max rescale numeric data to [0, 1].',
      },
      {
        name: 'split',
        params: ['data', 'frac', 'seed?'],
        doc: 'Seeded random train/test split of rows → struct(train, test).',
      },
      {
        name: 'standardize',
        params: ['v'],
        doc: 'Z-scores (vᵢ − μ)/σ as exact surds (sample σ).',
      },
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
    if (stream.match(/^(:=|==|!=|<=|>=|[+\-*/^=<>.~])/)) return 'operator'
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

/** Top-level fields of a struct, parsed from its workspace `text`, which the
 * engine renders as `struct(name = value, …)` (see Expr::Struct's render in
 * src/expr.rs). Bracket depth keeps nested structs, matrices, and parenthesized
 * equation-valued fields from leaking their own `name =` pairs or commas. */
function structFields(text: string): { name: string; value: string }[] {
  if (!text.startsWith('struct(') || !text.endsWith(')')) return []
  const inner = text.slice('struct('.length, -1)
  const fields: { name: string; value: string }[] = []
  let depth = 0
  let start = 0
  const push = (seg: string) => {
    const m = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=(?!=)\s*([\s\S]*)$/.exec(seg)
    if (m) fields.push({ name: m[1], value: m[2].trim() })
  }
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i]
    if (ch === '(' || ch === '[' || ch === '{') depth++
    else if (ch === ')' || ch === ']' || ch === '}') depth--
    else if (ch === ',' && depth === 0) {
      push(inner.slice(start, i))
      start = i + 1
    }
  }
  push(inner.slice(start))
  return fields
}

/** Builtins + keywords + whatever is bound in the live workspace. */
function completionSource(context: CompletionContext): CompletionResult | null {
  // Right of a dot, complete members: a namespace's functions, or — for a
  // struct bound in the workspace — its fields (and only those).
  const member = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z0-9_]*/)
  if (member) {
    const dot = member.text.indexOf('.')
    const qualifier = member.text.slice(0, dot)
    const ns = NAMESPACE_BY_NAME.get(qualifier)
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
    const entry = useNotebook
      .getState()
      .workspace.find((e) => e.name === qualifier && e.kind === 'struct')
    if (entry) {
      const fields = structFields(entry.text)
      if (fields.length) {
        return {
          from: member.from + dot + 1,
          options: fields.map((f) => {
            const value = f.value.replace(/\s+/g, ' ')
            return {
              label: f.name,
              type: 'property',
              detail: value.length > 24 ? value.slice(0, 24) + '…' : value,
            }
          }),
          validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
        }
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
