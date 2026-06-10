// Mirrors the serde JSON shapes in wasm/src/lib.rs. If those structs change,
// change these — there is no codegen, the smoke test (web/smoke.mjs) and the
// store tests are the tripwire.

/** A sampled point: y is null at poles / domain gaps. */
export type SamplePoint = [number, number | null]

export interface PlotData {
  /** LaTeX of the plotted expression, for the legend. */
  latex: string
  /** Re-parseable plain text of the expression (closed except `var`) — what
   * `resample` re-evaluates when the user pans/zooms. */
  text: string
  var: string
  a: number
  b: number
  points: SamplePoint[]
}

export type ResultKind =
  | 'scalar'
  | 'matrix'
  | 'boolean'
  | 'equation'
  | 'function'
  | 'plot'
  | 'error'

export interface EvalResult {
  ok: boolean
  kind: ResultKind
  text: string
  latex: string
  plot?: PlotData
  error?: string
}

export interface ResampleResult {
  ok: boolean
  points?: SamplePoint[]
  error?: string
}

/** One workspace binding, from Session.workspace(). */
export interface WorkspaceEntry {
  name: string
  text: string
  latex: string
  kind: ResultKind
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

export type ToWorker =
  | { type: 'init'; replay: string[] }
  | { type: 'eval'; id: number; src: string }
  | { type: 'workspace'; id: number }
  | {
      type: 'resample'
      id: number
      exprText: string
      varName: string
      a: number
      b: number
      n: number
    }

export type FromWorker =
  | { type: 'ready'; replayed: number }
  | { type: 'result'; id: number; result: EvalResult }
  | { type: 'workspace'; id: number; result: WorkspaceEntry[] }
  | { type: 'resampled'; id: number; result: ResampleResult }
