// Mirrors the serde JSON shapes in wasm/src/lib.rs. If those structs change,
// change these — there is no codegen, the smoke test (web/smoke.mjs) and the
// store tests are the tripwire.

/** A sampled point: y is null at poles / domain gaps. */
export type SamplePoint = [number, number | null]

/** One curve of a plot. */
export interface PlotSeries {
  /** LaTeX of the plotted expression, for the legend. */
  latex: string
  /** Re-parseable plain text of the expression (closed except `var`) — what
   * `resample` re-evaluates when the user pans/zooms. */
  text: string
  /** True when even the engine's finest adaptive resolution failed its
   * convergence test on this window — the curve may alias, and the UI says
   * so. Absent in pre-adaptive persisted notebooks (treat as false). */
  undersampled?: boolean
  /** True for static data series (signals, scatter): every point is already
   * present and `text` cannot be resampled — pan/zoom re-windows client-side. */
  fixed?: boolean
  /** True for scatter series: drawn as discrete markers instead of a connected
   * line. Always `fixed` too. Absent (treat as false) for curves and signals. */
  scatter?: boolean
  points: SamplePoint[]
}

export interface PlotData {
  var: string
  a: number
  b: number
  /** Session registry id for signal plots: zoom refinement re-decimates the
   * window from the original data via `resampleSignal`. Absent for function
   * plots (those resample by expression text). */
  sig?: number
  /** One entry per curve, over the shared [a, b] window. */
  series: PlotSeries[]
}

/** A surface z = f(x, y), sampled on a grid. */
export interface Plot3dData {
  latex: string
  text: string
  xvar: string
  a: number
  b: number
  yvar: string
  c: number
  d: number
  nx: number
  ny: number
  /** True when even the engine's finest adaptive grid failed its convergence
   * test on this window — the surface may alias, and the UI says so. Absent
   * in pre-adaptive persisted notebooks (treat as false). */
  undersampled?: boolean
  /** Row-major heights (y outer, x inner); null at poles / domain gaps.
   * Empty (with `nx` = 0) for a points-only plot. */
  heights: (number | null)[]
  /** 3D scatter markers `(x, y, z)` in data coordinates; absent when none.
   * Static data — re-windowed client-side, never resampled. */
  scatter?: [number, number, number][]
}

export type ResultKind =
  | 'scalar'
  | 'matrix'
  | 'boolean'
  | 'equation'
  | 'function'
  | 'struct'
  | 'plot'
  | 'plot3d'
  /** A data import's summary result (Session.import_data). */
  | 'data'
  | 'error'

export interface EvalResult {
  ok: boolean
  kind: ResultKind
  text: string
  latex: string
  plot?: PlotData
  plot3d?: Plot3dData
  error?: string
}

/** Pre-multi-curve persisted results: one points array + top-level
 * latex/text. Normalized at render time so saved notebooks keep painting. */
interface LegacyPlotData {
  latex: string
  text: string
  var: string
  a: number
  b: number
  points: SamplePoint[]
}

export function normalizePlotData(plot: PlotData | LegacyPlotData): PlotData {
  if ('series' in plot) return plot
  const { latex, text, points, ...window } = plot
  return { ...window, series: [{ latex, text, points }] }
}

export interface ResampleResult {
  ok: boolean
  points?: SamplePoint[]
  undersampled?: boolean
  error?: string
}

export interface Resample3dResult {
  ok: boolean
  heights?: (number | null)[]
  /** Grid resolution per axis the adaptive sampler settled on. */
  n?: number
  undersampled?: boolean
  error?: string
}

/** One workspace binding, from Session.workspace(). */
export interface WorkspaceEntry {
  name: string
  text: string
  latex: string
  kind: ResultKind
}

/** Result of Session.export_data: the surd-data file's text, or an error. */
export interface ExportResult {
  ok: boolean
  data?: string
  error?: string
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

/** How an import payload should be parsed. `auto` is the text sniffing path
 * (surd-data JSON / generic JSON / CSV → exact values); the rest are bulk
 * signal imports — `wav` and the `raw-*` formats carry base64-encoded bytes
 * in `payload`. */
export type ImportFormat =
  | 'auto'
  | 'wav'
  | 'raw-f64'
  | 'raw-f32'
  | 'raw-i16'
  | 'csv-packed'

/** One replayable step of a notebook's engine history: an evaluated source
 * line, or a raw-data import bound to a workspace name. */
export type ReplayEntry =
  | { type: 'eval'; src: string }
  | { type: 'import'; name: string; payload: string; format?: ImportFormat }

export type ToWorker =
  | { type: 'init'; replay: ReplayEntry[] }
  | { type: 'eval'; id: number; src: string }
  | {
      type: 'importData'
      id: number
      name: string
      payload: string
      format?: ImportFormat
    }
  | { type: 'exportData'; id: number; names: string[] }
  | { type: 'workspace'; id: number }
  | {
      type: 'resampleSignal'
      id: number
      sig: number
      series: number
      a: number
      b: number
    }
  | {
      type: 'resample'
      id: number
      exprText: string
      varName: string
      a: number
      b: number
    }
  | {
      type: 'resample3d'
      id: number
      exprText: string
      xvar: string
      yvar: string
      a: number
      b: number
      c: number
      d: number
    }

export type FromWorker =
  | { type: 'ready'; replayed: number }
  | { type: 'result'; id: number; result: EvalResult }
  | { type: 'imported'; id: number; result: EvalResult }
  | { type: 'exported'; id: number; result: ExportResult }
  | { type: 'workspace'; id: number; result: WorkspaceEntry[] }
  | { type: 'resampled'; id: number; result: ResampleResult }
  | { type: 'resampled3d'; id: number; result: Resample3dResult }
