// Worker that hosts one engine Session. The worker IS the cancellation
// boundary: the main thread terminates a stuck worker and spawns a fresh one,
// replaying the transcript to rebuild the workspace (the engine is
// deterministic, so the transcript is the serialized state).

import init, { Session } from './pkg/surd_wasm.js';

let session = null;

self.onmessage = async (e) => {
  const msg = e.data;
  switch (msg.type) {
    case 'init': {
      await init();
      session = new Session();
      for (const src of msg.replay) {
        session.eval(src); // rebuild workspace; results already rendered
      }
      postMessage({ type: 'ready', replayed: msg.replay.length });
      break;
    }
    case 'eval': {
      const result = JSON.parse(session.eval(msg.src));
      postMessage({ type: 'result', id: msg.id, result });
      break;
    }
  }
};
