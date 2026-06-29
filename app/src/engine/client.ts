// Promise-facing wrapper around the engine worker. Framework-agnostic — the
// Zustand store is its only consumer, React never touches it directly.
//
// Cancellation model: there is no in-band interrupt (the engine is synchronous
// wasm). `restart(transcript)` terminates the worker, rejects everything
// in flight with EngineCancelled, and boots a replacement that replays the
// transcript to rebuild the workspace.

import type {
  EvalResult,
  ExportResult,
  FromWorker,
  RawExportFormat,
  ReplayEntry,
  Resample3dResult,
  ResampleResult,
  SamplePoint,
  ToWorker,
  WorkspaceEntry,
  ImportFormat,
} from './types'

export class EngineCancelled extends Error {
  constructor() {
    super('evaluation cancelled')
    this.name = 'EngineCancelled'
  }
}

/** A resampled curve: points at the resolution the adaptive sampler settled
 * on, plus the engine's honesty flag (see PlotSeries.undersampled). */
export interface SampledCurve {
  points: SamplePoint[]
  undersampled: boolean
}

/** A resampled surface: heights on the n×n grid the adaptive sampler settled
 * on, plus the engine's honesty flag (see Plot3dData.undersampled). */
export interface Sampled3d {
  heights: (number | null)[]
  n: number
  undersampled: boolean
}

interface Pending {
  resolve: (v: never) => void
  reject: (e: Error) => void
}

export class EngineClient {
  private worker: Worker | null = null
  private nextId = 1
  private pending = new Map<number, Pending>()
  private readyPromise: Promise<number> | null = null

  /** Boot (or reboot) the worker, replaying `transcript` (evaluated sources
   * and data imports) into a fresh workspace. Resolves with the number of
   * replayed entries. */
  restart(transcript: ReplayEntry[]): Promise<number> {
    this.worker?.terminate()
    for (const p of this.pending.values()) p.reject(new EngineCancelled())
    this.pending.clear()

    const worker = new Worker(new URL('./worker.ts', import.meta.url), {
      type: 'module',
    })
    this.worker = worker
    this.readyPromise = new Promise((resolve, reject) => {
      worker.onmessage = (e: MessageEvent<FromWorker>) => {
        if (e.data.type === 'ready') {
          worker.onmessage = (ev: MessageEvent<FromWorker>) =>
            this.dispatch(ev.data)
          resolve(e.data.replayed)
        }
      }
      worker.onerror = (e) =>
        reject(new Error(`engine worker failed: ${e.message}`))
    })
    this.post({ type: 'init', replay: transcript })
    return this.readyPromise
  }

  /** Evaluate one complete statement block. One eval at a time is enforced by
   * the store; the worker would queue extras safely anyway. */
  eval(src: string): Promise<EvalResult> {
    return this.request<EvalResult>((id) => ({ type: 'eval', id, src }))
  }

  /** Import a data file, binding it to `name` in the workspace. `payload`
   * is the file's text — or base64 bytes for the binary formats. */
  importData(
    name: string,
    payload: string,
    format: ImportFormat = 'auto',
  ): Promise<EvalResult> {
    return this.request<EvalResult>((id) => ({
      type: 'importData',
      id,
      name,
      payload,
      format,
    }))
  }

  /** Serialize the named workspace variables into one surd-data file. */
  async exportData(names: string[]): Promise<string> {
    const r = await this.request<ExportResult>((id) => ({
      type: 'exportData',
      id,
      names,
    }))
    if (!r.ok || r.data === undefined)
      throw new Error(r.error ?? 'export failed')
    return r.data
  }

  /** Export one workspace variable as raw little-endian binary; resolves to
   * the base64 of the bytes (decoded by the save path). */
  async exportRaw(name: string, format: RawExportFormat): Promise<string> {
    const r = await this.request<ExportResult>((id) => ({
      type: 'exportRaw',
      id,
      name,
      format,
    }))
    if (!r.ok || r.data === undefined)
      throw new Error(r.error ?? 'export failed')
    return r.data
  }

  /** Current workspace bindings, for the variables panel. */
  workspace(): Promise<WorkspaceEntry[]> {
    return this.request<WorkspaceEntry[]>((id) => ({ type: 'workspace', id }))
  }

  /** Re-sample a plot expression over a new window (pan/zoom). Stateless on
   * the engine side; queues behind any running eval. The engine's resolution
   * is adaptive, so the result carries the honesty flag. */
  async resample(
    exprText: string,
    varName: string,
    a: number,
    b: number,
  ): Promise<SampledCurve> {
    const r = await this.request<ResampleResult>((id) => ({
      type: 'resample',
      id,
      exprText,
      varName,
      a,
      b,
    }))
    if (!r.ok || !r.points) throw new Error(r.error ?? 'resample failed')
    return { points: r.points, undersampled: r.undersampled ?? false }
  }

  /** Re-decimate one series of a registered signal plot over a new index
   * window (pan/zoom). Throws when the plot is no longer registered (session
   * restarted, registry evicted) — the caller keeps its shipped envelope. */
  async resampleSignal(
    sig: number,
    series: number,
    a: number,
    b: number,
  ): Promise<SampledCurve> {
    const r = await this.request<ResampleResult>((id) => ({
      type: 'resampleSignal',
      id,
      sig,
      series,
      a,
      b,
    }))
    if (!r.ok || !r.points) throw new Error(r.error ?? 'resample failed')
    return { points: r.points, undersampled: r.undersampled ?? false }
  }

  /** Re-sample a surface expression over a new [a, b]×[c, d] domain
   * (pan/zoom). Stateless, like resample. The engine's grid is adaptive, so
   * the result carries the resolution it settled on plus the honesty flag. */
  async resample3d(
    exprText: string,
    xvar: string,
    yvar: string,
    a: number,
    b: number,
    c: number,
    d: number,
  ): Promise<Sampled3d> {
    const r = await this.request<Resample3dResult>((id) => ({
      type: 'resample3d',
      id,
      exprText,
      xvar,
      yvar,
      a,
      b,
      c,
      d,
    }))
    if (!r.ok || !r.heights || !r.n)
      throw new Error(r.error ?? 'resample3d failed')
    return {
      heights: r.heights,
      n: r.n,
      undersampled: r.undersampled ?? false,
    }
  }

  private request<T>(make: (id: number) => ToWorker): Promise<T> {
    const id = this.nextId++
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: never) => void, reject })
      this.post(make(id))
    })
  }

  private dispatch(msg: FromWorker) {
    if (msg.type === 'ready') return
    const p = this.pending.get(msg.id)
    if (!p) return
    this.pending.delete(msg.id)
    p.resolve(msg.result as never)
  }

  private post(msg: ToWorker) {
    this.worker?.postMessage(msg)
  }
}
