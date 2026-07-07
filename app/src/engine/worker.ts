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
  ImportFormat,
  Resample3dResult,
  ResampleResult,
  ToWorker,
  WorkspaceEntry,
} from './types'

let session: Session | null = null

const post = (m: FromWorker) => self.postMessage(m)

/** Decode a base64 payload into bytes (bulk binary imports ride the
 * transcript as base64 so replay entries stay structured-cloneable JSON). */
function fromBase64(payload: string): Uint8Array {
  const bin = atob(payload)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes
}

/** Run one import in whichever format it carries. */
function runImport(
  s: Session,
  name: string,
  payload: string,
  format: ImportFormat = 'auto',
): string {
  switch (format) {
    case 'wav':
      return s.import_wav_data(fromBase64(payload), name)
    case 'mat':
      return s.import_mat_data(fromBase64(payload), name)
    case 'raw-f64':
    case 'raw-f32':
    case 'raw-i16':
      return s.import_raw_data(fromBase64(payload), format.slice(4), name)
    case 'raw-cf32':
    case 'raw-cf64':
      return s.import_raw_iq_data(fromBase64(payload), format.slice(4), name)
    case 'csv-packed':
      return s.import_csv_packed_data(payload, name)
    default:
      return s.import_data(payload, name)
  }
}

self.onmessage = async (e: MessageEvent<ToWorker>) => {
  const msg = e.data
  switch (msg.type) {
    case 'init': {
      await init({ module_or_path: wasmUrl })
      session = new Session()
      for (const entry of msg.replay) {
        // rebuild workspace; results were already rendered
        if (entry.type === 'eval') session.eval(entry.src)
        else runImport(session, entry.name, entry.payload, entry.format)
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
        runImport(session!, msg.name, msg.payload, msg.format),
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
    case 'exportRaw': {
      const result = JSON.parse(
        session!.export_raw(msg.name, msg.format),
      ) as ExportResult
      post({ type: 'exported', id: msg.id, result })
      break
    }
    case 'workspace': {
      const result = JSON.parse(session!.workspace()) as WorkspaceEntry[]
      post({ type: 'workspace', id: msg.id, result })
      break
    }
    case 'resampleSignal': {
      const result = JSON.parse(
        session!.resample_signal(msg.sig, msg.series, msg.a, msg.b),
      ) as ResampleResult
      post({ type: 'resampled', id: msg.id, result })
      break
    }
    case 'resample': {
      const result = JSON.parse(
        resample(msg.exprText, msg.varName, msg.a, msg.b),
      ) as ResampleResult
      post({ type: 'resampled', id: msg.id, result })
      break
    }
    case 'resample3d': {
      const result = JSON.parse(
        resample3d(
          msg.exprText,
          msg.xvar,
          msg.yvar,
          msg.a,
          msg.b,
          msg.c,
          msg.d,
        ),
      ) as Resample3dResult
      post({ type: 'resampled3d', id: msg.id, result })
      break
    }
  }
}
