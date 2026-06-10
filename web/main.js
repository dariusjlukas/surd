// Main-thread UI for the exact scratchpad.
//
// - The engine runs in a worker; this thread only renders.
// - Cancel = terminate the worker + replay the saved transcript (the engine is
//   deterministic, so the transcript of successful inputs IS the workspace).
// - The wasm module is also loaded here, but only for is_incomplete/is_blank —
//   continuation detection must not queue behind a long-running eval.

import init, { is_incomplete, is_blank } from './pkg/exact_wasm.js';

const TRANSCRIPT_KEY = 'exact.transcript.v1';

const history = document.getElementById('history');
const input = document.getElementById('input');
const prompt = document.getElementById('prompt');
const status = document.getElementById('status');
const cancelBtn = document.getElementById('cancel');
const clearBtn = document.getElementById('clear');

let worker = null;
let busy = false;
let buffer = [];           // lines of an incomplete block
let transcript = [];       // successful inputs, replayed on restart
let inputLog = [];         // for ↑/↓ recall
let recall = -1;
let evalId = 0;
let pendingEntry = null;   // DOM node of the in-flight evaluation

function loadTranscript() {
  try {
    return JSON.parse(localStorage.getItem(TRANSCRIPT_KEY) || '[]');
  } catch {
    return [];
  }
}

function saveTranscript() {
  localStorage.setItem(TRANSCRIPT_KEY, JSON.stringify(transcript));
}

function setStatus(text, withCancel) {
  status.textContent = text;
  cancelBtn.style.display = withCancel ? 'inline-block' : 'none';
}

function startWorker(replay) {
  worker = new Worker('./worker.js', { type: 'module' });
  busy = true;
  setStatus(replay.length ? `restoring workspace (${replay.length} statements)…` : 'loading engine…', false);
  worker.postMessage({ type: 'init', replay });
  worker.onmessage = (e) => {
    const msg = e.data;
    if (msg.type === 'ready') {
      busy = false;
      setStatus('ready', false);
      input.focus();
    } else if (msg.type === 'result') {
      busy = false;
      setStatus('ready', false);
      renderResult(pendingEntry, msg.result);
      if (msg.result.ok) {
        transcript.push(pendingEntry.dataset.src);
        saveTranscript();
      }
      pendingEntry = null;
    }
  };
}

function cancelEvaluation() {
  if (!busy || !worker) return;
  worker.terminate();
  if (pendingEntry) {
    const out = pendingEntry.querySelector('.output');
    out.textContent = 'cancelled';
    out.className = 'output error';
    pendingEntry = null;
  }
  startWorker(transcript); // rebuild workspace from the transcript
}

function submit(src) {
  const entry = document.createElement('div');
  entry.className = 'entry';
  entry.dataset.src = src;
  const inputEl = document.createElement('pre');
  inputEl.className = 'echo';
  inputEl.textContent = src;
  const out = document.createElement('div');
  out.className = 'output running';
  out.textContent = '…';
  entry.append(inputEl, out);
  history.append(entry);
  entry.scrollIntoView({ block: 'end' });

  pendingEntry = entry;
  busy = true;
  setStatus('evaluating… ', true);
  worker.postMessage({ type: 'eval', id: ++evalId, src });
}

function renderResult(entry, result) {
  if (!entry) return;
  const out = entry.querySelector('.output');
  out.classList.remove('running');
  if (!result.ok) {
    out.textContent = `error: ${result.error}`;
    out.classList.add('error');
    return;
  }
  if (result.kind === 'plot') {
    out.textContent = '';
    out.append(drawPlot(result.plot));
    return;
  }
  if (result.kind === 'function') {
    out.textContent = result.text; // "<function(n)>" — not math
    return;
  }
  try {
    katex.render(result.latex, out, { throwOnError: true, displayMode: true });
  } catch {
    out.textContent = result.text; // fallback if KaTeX chokes
  }
  out.title = result.text; // hover shows the re-parseable plain form
}

// ---------------------------------------------------------------------------
// Plot rendering: canvas polyline with gaps at nulls, robust y-scaling so a
// pole doesn't flatten the rest of the curve.
// ---------------------------------------------------------------------------

function drawPlot(plot) {
  const box = document.createElement('div');
  box.className = 'plot';
  const label = document.createElement('div');
  label.className = 'plot-label';
  try {
    katex.render(plot.latex, label, { throwOnError: true });
  } catch {
    label.textContent = plot.latex;
  }
  const canvas = document.createElement('canvas');
  const cssW = 640, cssH = 360;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = cssW * dpr;
  canvas.height = cssH * dpr;
  canvas.style.width = cssW + 'px';
  canvas.style.height = cssH + 'px';
  const ctx = canvas.getContext('2d');
  ctx.scale(dpr, dpr);

  const ys = plot.points.map(p => p[1]).filter(y => y !== null).sort((a, b) => a - b);
  // 2%–98% quantiles: a pole's spike must not flatten everything else.
  let lo = ys[Math.floor(ys.length * 0.02)];
  let hi = ys[Math.min(ys.length - 1, Math.floor(ys.length * 0.98))];
  if (lo === hi) { lo -= 1; hi += 1; }
  const pad = (hi - lo) * 0.08;
  lo -= pad; hi += pad;

  const m = { l: 46, r: 12, t: 10, b: 24 };
  const px = x => m.l + (x - plot.a) / (plot.b - plot.a) * (cssW - m.l - m.r);
  const py = y => cssH - m.b - (y - lo) / (hi - lo) * (cssH - m.t - m.b);

  // frame + zero lines
  ctx.strokeStyle = '#3b4252';
  ctx.lineWidth = 1;
  ctx.strokeRect(m.l, m.t, cssW - m.l - m.r, cssH - m.t - m.b);
  ctx.strokeStyle = '#4c566a';
  if (lo < 0 && hi > 0) {
    ctx.beginPath(); ctx.moveTo(m.l, py(0)); ctx.lineTo(cssW - m.r, py(0)); ctx.stroke();
  }
  if (plot.a < 0 && plot.b > 0) {
    ctx.beginPath(); ctx.moveTo(px(0), m.t); ctx.lineTo(px(0), cssH - m.b); ctx.stroke();
  }

  // axis extent labels
  ctx.fillStyle = '#9aa3b2';
  ctx.font = '11px ui-monospace, monospace';
  ctx.textAlign = 'left';
  ctx.fillText(fmtTick(plot.a), m.l, cssH - 8);
  ctx.textAlign = 'right';
  ctx.fillText(fmtTick(plot.b), cssW - m.r, cssH - 8);
  ctx.fillText(fmtTick(hi), m.l - 4, m.t + 10);
  ctx.fillText(fmtTick(lo), m.l - 4, cssH - m.b);

  // the curve, clipped to the frame, broken at gaps
  ctx.save();
  ctx.beginPath();
  ctx.rect(m.l, m.t, cssW - m.l - m.r, cssH - m.t - m.b);
  ctx.clip();
  ctx.strokeStyle = '#88c0d0';
  ctx.lineWidth = 1.6;
  ctx.beginPath();
  let pen = false;
  for (const [x, y] of plot.points) {
    if (y === null) { pen = false; continue; }
    if (pen) ctx.lineTo(px(x), py(y));
    else ctx.moveTo(px(x), py(y));
    pen = true;
  }
  ctx.stroke();
  ctx.restore();

  box.append(label, canvas);
  return box;
}

function fmtTick(v) {
  if (v === 0) return '0';
  const a = Math.abs(v);
  if (a >= 1000 || a < 0.01) return v.toExponential(1);
  return String(Math.round(v * 100) / 100);
}

// ---------------------------------------------------------------------------
// Input handling: REPL line discipline with block continuation
// ---------------------------------------------------------------------------

input.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    e.preventDefault();
    if (busy) return;
    const line = input.value;
    input.value = '';
    buffer.push(line);
    const src = buffer.join('\n');
    if (is_incomplete(src)) {
      prompt.textContent = '..';
      return;
    }
    buffer = [];
    prompt.textContent = '>>';
    if (is_blank(src)) return;
    inputLog.push(src);
    recall = -1;
    submit(src);
  } else if (e.key === 'ArrowUp' && buffer.length === 0) {
    if (inputLog.length === 0) return;
    e.preventDefault();
    recall = recall < 0 ? inputLog.length - 1 : Math.max(0, recall - 1);
    input.value = inputLog[recall].split('\n')[0];
  } else if (e.key === 'ArrowDown' && recall >= 0) {
    e.preventDefault();
    recall += 1;
    if (recall >= inputLog.length) { recall = -1; input.value = ''; }
    else input.value = inputLog[recall].split('\n')[0];
  } else if (e.key === 'Escape' && buffer.length > 0) {
    buffer = [];
    prompt.textContent = '>>';
    setStatus('block cancelled', false);
  }
});

cancelBtn.addEventListener('click', cancelEvaluation);

clearBtn.addEventListener('click', () => {
  if (!confirm('Clear the saved workspace and history?')) return;
  transcript = [];
  saveTranscript();
  history.innerHTML = '';
  if (worker) worker.terminate();
  startWorker([]);
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

await init(); // main-thread copy, for is_incomplete/is_blank only
transcript = loadTranscript();
inputLog = [...transcript];
if (transcript.length) {
  const note = document.createElement('div');
  note.className = 'note';
  note.textContent = `restored workspace from ${transcript.length} saved statement(s) — type :clear or use the button to reset`;
  history.append(note);
}
startWorker(transcript);
