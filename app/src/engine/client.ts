// Promise-facing wrapper around the engine worker. Framework-agnostic — the
// Zustand store is its only consumer, React never touches it directly.
//
// Cancellation model: there is no in-band interrupt (the engine is synchronous
// wasm). `restart(transcript)` terminates the worker, rejects everything
// in flight with EngineCancelled, and boots a replacement that replays the
// transcript to rebuild the workspace.

import type {
  EvalResult,
  FromWorker,
  ResampleResult,
  SamplePoint,
  ToWorker,
  WorkspaceEntry,
} from './types'

export class EngineCancelled extends Error {
  constructor() {
    super('evaluation cancelled')
    this.name = 'EngineCancelled'
  }
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

  /** Boot (or reboot) the worker, replaying `transcript` into a fresh
   * workspace. Resolves with the number of replayed statements. */
  restart(transcript: string[]): Promise<number> {
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
          worker.onmessage = (ev: MessageEvent<FromWorker>) => this.dispatch(ev.data)
          resolve(e.data.replayed)
        }
      }
      worker.onerror = (e) => reject(new Error(`engine worker failed: ${e.message}`))
    })
    this.post({ type: 'init', replay: transcript })
    return this.readyPromise
  }

  /** Evaluate one complete statement block. One eval at a time is enforced by
   * the store; the worker would queue extras safely anyway. */
  eval(src: string): Promise<EvalResult> {
    return this.request<EvalResult>((id) => ({ type: 'eval', id, src }))
  }

  /** Current workspace bindings, for the variables panel. */
  workspace(): Promise<WorkspaceEntry[]> {
    return this.request<WorkspaceEntry[]>((id) => ({ type: 'workspace', id }))
  }

  /** Re-sample a plot expression over a new window (pan/zoom). Stateless on
   * the engine side; queues behind any running eval. */
  async resample(
    exprText: string,
    varName: string,
    a: number,
    b: number,
    n = 600,
  ): Promise<SamplePoint[]> {
    const r = await this.request<ResampleResult>((id) => ({
      type: 'resample',
      id,
      exprText,
      varName,
      a,
      b,
      n,
    }))
    if (!r.ok || !r.points) throw new Error(r.error ?? 'resample failed')
    return r.points
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
