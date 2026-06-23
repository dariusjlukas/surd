// Mirror of `latex_symbol` in src/latex.rs: turns a bare identifier into the
// LaTeX a math renderer expects, so variable NAMES (workspace rows, plot axis
// labels) typeset their Greek letters and subscripts the same way the engine
// already renders symbols INSIDE expressions. There is no codegen — if the Rust
// side changes, change this too (same convention as engine/types.ts).

// Every Greek LaTeX command is a backslash before the name, so one set serves
// the lookup for lower-, upper-case, and the `var*` variants alike.
const GREEK = new Set([
  // lowercase
  'alpha',
  'beta',
  'gamma',
  'delta',
  'epsilon',
  'zeta',
  'eta',
  'theta',
  'iota',
  'kappa',
  'lambda',
  'mu',
  'nu',
  'xi',
  'pi',
  'rho',
  'sigma',
  'tau',
  'upsilon',
  'phi',
  'chi',
  'psi',
  'omega',
  // lowercase variants
  'varepsilon',
  'vartheta',
  'varpi',
  'varphi',
  'varrho',
  'varsigma',
  'varkappa',
  // uppercase (only those with a distinct glyph / command)
  'Gamma',
  'Delta',
  'Theta',
  'Lambda',
  'Xi',
  'Pi',
  'Sigma',
  'Upsilon',
  'Phi',
  'Psi',
  'Omega',
])

/** One underscore-free token → its LaTeX atom: a Greek name to its command, a
 *  lone character as-is, anything else upright. */
function atom(s: string): string {
  if (GREEK.has(s)) return `\\${s}`
  if ([...s].length === 1) return s
  // The escape only bites in the degenerate fallback (clean splits never hand
  // an underscore-bearing token here).
  return `\\mathrm{${s.replace(/_/g, '\\_')}}`
}

/** A bare identifier → LaTeX. `beta_0` → `\beta_{0}`, `x_i_j` → `x_{i_{j}}`,
 *  `v_max` → `v_{\mathrm{max}}`. A degenerate name (a leading, trailing, or
 *  doubled underscore leaves an empty segment) falls back to one upright token
 *  so KaTeX never chokes on a bare `_`. */
export function nameToLatex(name: string): string {
  const parts = name.split('_')
  if (parts.length === 1 || parts.some((p) => p === '')) return atom(name)
  // Build the subscript chain from the inside out: a_b_c → a_{b_{c}}.
  let acc = atom(parts[parts.length - 1])
  for (let i = parts.length - 2; i >= 0; i--) {
    acc = `${atom(parts[i])}_{${acc}}`
  }
  return acc
}
