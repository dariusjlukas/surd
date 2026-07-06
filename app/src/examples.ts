// Built-in example notebooks for the Welcome empty state. Sources only —
// opening one evaluates the cells live on the user's own engine
// (store.openExample), so every result, badge, and refusal on screen is
// real, never canned. Each math line here is verified against the REPL;
// if you change one, re-run it there first.

export interface ExampleCell {
  kind: 'math' | 'markdown'
  src: string
}

export interface Example {
  name: string
  /** One-line pitch shown next to the name in the Welcome gallery. */
  blurb: string
  cells: ExampleCell[]
}

const md = (src: string): ExampleCell => ({ kind: 'markdown', src })
const math = (src: string): ExampleCell => ({ kind: 'math', src })

export const EXAMPLES: Example[] = [
  {
    name: 'Tour: exact by default',
    blurb: 'rationals, radicals, proofs — and an honest refusal',
    cells: [
      md(
        '# Exact by default\n\n' +
          'surd never rounds silently. Every result carries a badge — ' +
          '**exact**, **symbolic**, **approximate**, or **certified** — and ' +
          'exact values show a faint `≈` preview whose digits are *proven* ' +
          'correct, not floated.',
      ),
      math('1/3 + 1/6'),
      math('phi := (1 + sqrt(5))/2'),
      math('phi^2 - phi'),
      math('N(pi, 50)'),
      math('fact(n) := if n == 0 then 1 else n*fact(n-1) end'),
      math('fact(50)'),
      math('map(x -> x^2, [1, 2, 3])'),
      md(
        '## Comparisons are proofs\n\n' +
          'A comparison only answers when the engine can *certify* the ' +
          'answer — by exact algebra or by interval refinement that provably ' +
          'separates the two sides.',
      ),
      math('phi^2 == phi + 1'),
      math('sqrt(2) + sqrt(3) > pi'),
      math('sin(1)^2 + cos(1)^2 == 1'),
      md(
        'That last one is surd working as designed: the two sides agree to ' +
          'thousands of digits (the identity *is* exactly true), but proving ' +
          'it needs trig identities outside the certified classes — so surd ' +
          '**refuses rather than guesses**. A refusal is never a wrong answer.',
      ),
      math('plot(sin(x)/x, x, -15, 15, title = "the sinc function")'),
    ],
  },
  {
    name: 'Certified DSP',
    blurb: 'signals with proven error bounds, exact filter design',
    cells: [
      md(
        '# Certified signal processing\n\n' +
          'Bulk numeric data lives in *signals*: every sample carries a ' +
          'proven error enclosure, and `bound(s)` reports the certified ' +
          'worst case. Exact input data starts with bound 0.',
      ),
      math('s := signal([1; 2; 3; 4; 5; 6; 7; 8])'),
      math('bound(s)'),
      math('r := re(dsp.ifft(dsp.fft(s)));'),
      math('dsp.peak(r - s) < 1/10^12'),
      md(
        'That `true` is a theorem about *this* run: the FFT round-trip ' +
          'error is provably below 10⁻¹² — certified, not eyeballed.',
      ),
      math('dsp.window(hann, 8)'),
      md(
        '## Exact filter design\n\n' +
          '`dsp.remez` solves Parks–McClellan **exactly**: no convergence ' +
          'failures, and the returned ripple is an exact rational — so ' +
          'checking a spec is a decidable comparison, not an eyeball.',
      ),
      math('f := dsp.remez(15, [0, 2/5*pi, 1/2*pi, pi], [1, 0]);'),
      math('N(f.ripple, 6)'),
      math('abs(dsp.freqz(f.taps, [pi])[1]) <= f.ripple'),
    ],
  },
  {
    name: 'Statistics, exactly',
    blurb: 'exact fits and correlations; digits only when you ask',
    cells: [
      md(
        '# Statistics without float drift\n\n' +
          'Descriptive statistics and fits run on exact rationals; ' +
          'p-values and CDFs stay symbolic until you ask for digits with ' +
          '`N(...)` — approximation is always opt-in.',
      ),
      math('x := [1; 2; 3; 4; 5; 6];'),
      math('y := [2; 4; 5; 4; 5; 7];'),
      math('stats.mean(y)'),
      math('stats.cor(x, y)'),
      math('fit := stats.linfit(x, y)'),
      math('fit.predict(10)'),
      math('N(stats.normcdf(2, 0, 1), 15)'),
      md(
        'Import your own data from the workspace panel: CSV columns arrive ' +
          'as exact values, and bulk signal formats (WAV, raw binary, I/Q) ' +
          'arrive as certified enclosures.',
      ),
    ],
  },
]
