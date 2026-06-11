// Main-thread copy of the wasm module, used ONLY for line discipline
// (is_incomplete / is_blank). Continuation detection must answer instantly,
// and the worker may be busy with a long evaluation — so the input bar gets
// its own engine instance that never evaluates anything.

import init, { is_blank, is_incomplete } from './pkg/surd_wasm'
import wasmUrl from './pkg/surd_wasm_bg.wasm?url'

let ready: Promise<void> | null = null

export function initLexer(): Promise<void> {
  ready ??= init({ module_or_path: wasmUrl }).then(() => undefined)
  return ready
}

export { is_blank, is_incomplete }
