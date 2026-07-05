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

/** A scatterplot matrix (SPLOM) from `pairs(...)`: k variables drawn as a k×k
 * grid of panels — lower triangle scatter, upper triangle correlation,
 * diagonal variable names. */
export interface SplomData {
  /** Variable labels, one per row and column of the panel grid. */
  labels: string[]
  /** k columns of decimated samples; null is a non-numeric / non-finite gap. */
  columns: (number | null)[][]
  /** [min, max] per variable — the shared scale down its column and across its
   * row, so every panel in a row/column reads on the same axis. */
  ranges: [number, number][]
  /** Row-major k×k Pearson r for the panel annotations; null where a variable
   * is constant (correlation undefined). */
  cor: (number | null)[]
  /** Samples drawn per variable after decimation, and the original count — the
   * UI notes when it's showing a thinned view. */
  shown: number
  total: number
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
  /** A scatterplot matrix (pairs(...)). */
  | 'splom'
  /** An STFT heatmap (spectrogram(...)). */
  | 'spectrogram'
  /** A data import's summary result (Session.import_data). */
  | 'data'
  | 'error'

/** An STFT heatmap prepared by the engine: dB·10 magnitudes on a pooled
 * display grid, row-major [frame][bin]. Mirrors wasm SpectrogramData. */
export interface SpectrogramData {
  db10: number[]
  frames: number
  bins: number
  /** Sample positions of the first/last frame center. */
  t_lo: number
  t_hi: number
  /** Frequency extent in units of π rad/sample ([0,1] real, [-1,1] complex). */
  f_lo: number
  f_hi: number
  /** Color range in dB (robust lower edge). */
  db_min: number
  db_max: number
  total_frames: number
  pooled: boolean
}

export interface EvalResult {
  ok: boolean
  kind: ResultKind
  text: string
  latex: string
  /** The input ended in `;` (MATLAB/Julia output suppression): the value was
   * computed and the workspace updated, but the cell renders compactly instead
   * of echoing a possibly-huge matrix. Absent (falsy) on older saved results. */
  suppressed?: boolean
  /** One-line shape hint for the compact rendering of a suppressed result,
   * e.g. `"5×3 matrix"` or `"8-vector"`. Present only when `suppressed`. */
  summary?: string
  plot?: PlotData
  plot3d?: Plot3dData
  splom?: SplomData
  spectrogram?: SpectrogramData
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
  /** Raw-binary export shape: `'real'` offers f32/f64, `'complex'` offers
   * cf32/cf64. Absent when the value can't export to raw binary. */
  raw?: 'real' | 'complex'
}

/** Result of Session.export_data: the surd-data file's text, or an error. */
export interface ExportResult {
  ok: boolean
  data?: string
  error?: string
}

/** Raw binary export precisions: `f32`/`f64` for real signals and numeric
 * data, `cf32`/`cf64` for interleaved I/Q (complex signals). */
export type RawExportFormat = 'f32' | 'f64' | 'cf32' | 'cf64'

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
  | 'raw-cf32'
  | 'raw-cf64'
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
  | { type: 'exportRaw'; id: number; name: string; format: RawExportFormat }
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
