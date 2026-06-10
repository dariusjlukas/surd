// The worker that hosts one engine Session. It is intentionally dumb: decode
// message, call wasm, post result. All policy (cancellation, replay,
// queueing) lives in EngineClient/the store on the main thread.
//
// The worker IS the cancellation boundary: EngineClient terminates it and
// spawns a fresh one, replaying the transcript (the engine is deterministic,
// so the transcript of successful inputs is the serialized workspace).

import init, { Session, resample } from './pkg/exact_wasm'
import wasmUrl from './pkg/exact_wasm_bg.wasm?url'
import type {
  EvalResult,
  FromWorker,
  ResampleResult,
  ToWorker,
  WorkspaceEntry,
} from './types'

let session: Session | null = null

const post = (m: FromWorker) => self.postMessage(m)

self.onmessage = async (e: MessageEvent<ToWorker>) => {
  const msg = e.data
  switch (msg.type) {
    case 'init': {
      await init({ module_or_path: wasmUrl })
      session = new Session()
      for (const src of msg.replay) {
        session.eval(src) // rebuild workspace; results were already rendered
      }
      post({ type: 'ready', replayed: msg.replay.length })
      break
    }
    case 'eval': {
      const result = JSON.parse(session!.eval(msg.src)) as EvalResult
      post({ type: 'result', id: msg.id, result })
      break
    }
    case 'workspace': {
      const result = JSON.parse(session!.workspace()) as WorkspaceEntry[]
      post({ type: 'workspace', id: msg.id, result })
      break
    }
    case 'resample': {
      const result = JSON.parse(
        resample(msg.exprText, msg.varName, msg.a, msg.b, msg.n),
      ) as ResampleResult
      post({ type: 'resampled', id: msg.id, result })
      break
    }
  }
}
