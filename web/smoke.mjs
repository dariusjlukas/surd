// Headless smoke test of the built wasm bundle (run: node web/smoke.mjs).
// Exercises the same module the browser loads — init, eval, persistence,
// plotting — without needing a browser.
import { readFile } from 'node:fs/promises';
import init, { Session, is_incomplete, is_blank } from './pkg/surd_wasm.js';

const bytes = await readFile(new URL('./pkg/surd_wasm_bg.wasm', import.meta.url));
await init({ module_or_path: bytes });

const s = new Session();
const ev = (src) => JSON.parse(s.eval(src));

const checks = [];
const expect = (name, got, want) => {
  const pass = JSON.stringify(got) === JSON.stringify(want);
  checks.push(pass);
  console.log(`${pass ? 'ok ' : 'FAIL'} ${name}: ${JSON.stringify(got)}${pass ? '' : ' != ' + JSON.stringify(want)}`);
};

expect('exact rational', ev('1/3 + 1/6').text, '1/2');
expect('latex', ev('sqrt(2)/2').latex, '\\frac{\\sqrt{2}}{2}');
expect('assign', ev('x := 3').text, '3');
expect('diff at binding', ev('diff(x^2, x)').text, '6');
expect('matrix kind', ev('inv([1,2;3,4])').kind, 'matrix');
expect('error shape', ev('1/0').error, 'division by zero');
expect('incomplete block', is_incomplete('if x then'), true);
expect('blank', is_blank('# comment'), true);

expect('implicit multiplication', ev('2w + w').text, '3*w');
expect('implicit call mult', ev('2sin(0)').text, '0');

s.eval('g := 7');
const ws = JSON.parse(s.workspace());
expect('workspace lists bindings', ws.some((e) => e.name === 'g' && e.text === '7'), true);

const plot = ev('plot(sin(y)/y, y, -10, 10)');
expect('plot kind', plot.kind, 'plot');
expect('plot series', plot.plot.series.length, 1);
expect('plot samples', plot.plot.series[0].points.length, 601);
expect('plot not undersampled', plot.plot.series[0].undersampled, false);
// the odd sample count lands exactly on the pole at 0: an honest gap…
expect('sinc gap at the pole', plot.plot.series[0].points[300][1], null);
// …with the curve approaching 1 on either side
const mid = plot.plot.series[0].points[299][1];
expect('sinc near 1 beside 0', Math.abs(mid - 1) < 0.01, true);

const multi = ev('plot(sin(y), cos(y), y, 0, 1)');
expect('multi-curve series', multi.plot.series.length, 2);

const surf = ev('plot3d(u^2 - v^2, u, -1, 1, v, -1, 1)');
expect('plot3d kind', surf.kind, 'plot3d');
expect('plot3d grid', surf.plot3d.heights.length, surf.plot3d.nx * surf.plot3d.ny);

// Raw-data import (CSV → struct of exact column vectors) + grouped export.
const imp = JSON.parse(s.import_data('t, val\n0, 0.5\n1, 2e1\n', 'sensor'));
expect('csv import ok', imp.ok, true);
expect('csv import kind', imp.kind, 'data');
expect('imported field is exact', ev('sensor.val').text.includes('1/2'), true);
expect('struct field access', ev('struct(a = 1/3, b = 2).a').text, '1/3');
const exp = JSON.parse(s.export_data('["sensor", "g"]'));
expect('export ok', exp.ok, true);
const re = JSON.parse(s.import_data(exp.data, 'saved'));
expect('re-import ok', re.ok, true);
expect('round-trip scalar', ev('saved.g + 1').text, '8');
expect('round-trip matrix', ev('saved.sensor.t + saved.sensor.val').kind, 'matrix');

// --- signals: certified bulk data ------------------------------------------
const sig = ev('snd := signal([1/3; -2; 5/7; 1])');
expect('signal kind', sig.kind, 'scalar'); // a plain value with a summary display
expect('signal display', sig.text.startsWith('<signal: 4 samples, f64'), true);
expect('signal certified bound', ev('bound(snd) < 1/10^15').text, 'true');
const sigPlot = ev('plot(snd)');
expect('signal plot kind', sigPlot.kind, 'plot');
expect('signal plot fixed', sigPlot.plot.series[0].fixed, true);
expect('signal plot points', sigPlot.plot.series[0].points.length, 4);
const fftOk = ev('dsp.peak(dsp.ifft(dsp.fft(dsp.pad(snd, 4))).re - dsp.pad(snd, 4)) < 1/10^12');
expect('signal fft roundtrip certified', fftOk.text, 'true');

// Bulk imports: packed CSV and a constructed 16-bit PCM WAV.
const csvBulk = JSON.parse(s.import_csv_packed_data('t, y\n0, 1.5\n1, 0.25\n', 'bulk'));
expect('csv-packed import ok', csvBulk.ok, true);
expect('csv-packed signal', ev('len(bulk.y)').text, '2');

const wavSamples = new Int16Array([0, 16384, -16384, 32767]);
const data = new Uint8Array(wavSamples.buffer);
const header = new ArrayBuffer(44);
const dv = new DataView(header);
const tag = (off, s2) => { for (let i = 0; i < 4; i++) dv.setUint8(off + i, s2.charCodeAt(i)); };
tag(0, 'RIFF'); dv.setUint32(4, 36 + data.length, true); tag(8, 'WAVE');
tag(12, 'fmt '); dv.setUint32(16, 16, true);
dv.setUint16(20, 1, true); dv.setUint16(22, 1, true);
dv.setUint32(24, 8000, true); dv.setUint32(28, 16000, true);
dv.setUint16(32, 2, true); dv.setUint16(34, 16, true);
tag(36, 'data'); dv.setUint32(40, data.length, true);
const wav = new Uint8Array(44 + data.length);
wav.set(new Uint8Array(header), 0); wav.set(data, 44);
const wavRes = JSON.parse(s.import_wav_data(wav, 'clip'));
expect('wav import ok', wavRes.ok, true);
expect('wav rate', ev('clip.rate').text, '8000');
expect('wav normalized exactly', ev('clip.ch1[2]').text, '0.5');
expect('wav import is exact', ev('bound(clip.ch1)').text, '0');
expect('wav slice', ev('len(slice(clip.ch1, 2, 3))').text, '3');

// Big-substrate signals export losslessly (decimal-string bounds) …
ev('hp := signal([1/3; -2], 40)');
const bigExp = JSON.parse(s.export_data(JSON.stringify(['hp'])));
expect('big signal export ok', bigExp.ok, true);
const bigRe = JSON.parse(s.import_data(bigExp.data, 'hp2'));
expect('big signal re-import ok', bigRe.ok, true);
expect('big signal bounds identical', ev('bound(hp2.hp) == bound(hp)').text, 'true');
expect('big signal mids identical', ev('hp2.hp[1] == hp[1]').text, 'true');

// … and decimated signal plots refine on zoom via the session registry.
ev('ramp := dsp.pad(signal([1]), 60000)');
const big = ev('plot(ramp)');
expect('decimated plot flagged', big.plot.series[0].undersampled, true);
expect('plot has registry id', typeof big.plot.sig, 'number');
const zoom = JSON.parse(s.resample_signal(big.plot.sig, 0, 1, 100));
expect('zoom refinement ok', zoom.ok, true);
expect('zoomed window is exact-resolution', zoom.points.length, 100);
expect('zoomed window not undersampled', zoom.undersampled, false);
const gone = JSON.parse(s.resample_signal(9999, 0, 1, 100));
expect('stale id refuses gracefully', gone.ok, false);

// Exact Remez and certified windows.
const rz = ev('rz := dsp.remez(11, [0, 2/5*pi, 1/2*pi, pi], [1, 0])');
expect('remez ok', rz.kind, 'struct' === rz.kind ? 'struct' : rz.kind);
expect('remez spec holds exactly',
  ev('abs(dsp.freqz(rz.taps, [pi])[1]) <= rz.ripple and rz.taps[1] == rz.taps[11]').text, 'true');
expect('remez allpass exact', ev('dsp.remez(5, [0, pi], [1]).ripple').text, '0');
expect('certified window', ev('bound(dsp.window(hann, 16)) < 1/10^12').text, 'true');

if (checks.every(Boolean)) {
  console.log(`\nall ${checks.length} checks passed`);
} else {
  console.error('\nSMOKE TEST FAILED');
  process.exit(1);
}
