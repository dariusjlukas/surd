// The worker that hosts one engine Session. It is intentionally dumb: decode
// message, call wasm, post result. All policy (cancellation, replay,
// queueing) lives in EngineClient/the store on the main thread.
//
// The worker IS the cancellation boundary: EngineClient terminates it and
// spawns a fresh one, replaying the transcript (the engine is deterministic,
// so the transcript of successful inputs is the serialized workspace).

import init, { Session, resample, resample3d } from './pkg/surd_wasm'
import wasmUrl from './pkg/surd_wasm_bg.wasm?url'
import type {
  EvalResult,
  ExportResult,
  FromWorker,
  Resample3dResult,
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
      for (const entry of msg.replay) {
        // rebuild workspace; results were already rendered
        if (entry.type === 'eval') session.eval(entry.src)
        else session.import_data(entry.payload, entry.name)
      }
      post({ type: 'ready', replayed: msg.replay.length })
      break
    }
    case 'eval': {
      const result = JSON.parse(session!.eval(msg.src)) as EvalResult
      post({ type: 'result', id: msg.id, result })
      break
    }
    case 'importData': {
      const result = JSON.parse(
        session!.import_data(msg.payload, msg.name),
      ) as EvalResult
      post({ type: 'imported', id: msg.id, result })
      break
    }
    case 'exportData': {
      const result = JSON.parse(
        session!.export_data(JSON.stringify(msg.names)),
      ) as ExportResult
      post({ type: 'exported', id: msg.id, result })
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
    case 'resample3d': {
      const result = JSON.parse(
        resample3d(msg.exprText, msg.xvar, msg.yvar, msg.a, msg.b, msg.c, msg.d, msg.n),
      ) as Resample3dResult
      post({ type: 'resampled3d', id: msg.id, result })
      break
    }
  }
}
