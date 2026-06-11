// Headless smoke test of the built wasm bundle (run: node web/smoke.mjs).
// Exercises the same module the browser loads — init, eval, persistence,
// plotting — without needing a browser.
import { readFile } from 'node:fs/promises';
import init, { Session, is_incomplete, is_blank } from './pkg/exact_wasm.js';

const bytes = await readFile(new URL('./pkg/exact_wasm_bg.wasm', import.meta.url));
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
expect('plot samples', plot.plot.series[0].points.length, 600);
const mid = plot.plot.series[0].points[300][1];
expect('sinc near 1 at 0', Math.abs(mid - 1) < 0.01, true);

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

if (checks.every(Boolean)) {
  console.log(`\nall ${checks.length} checks passed`);
} else {
  console.error('\nSMOKE TEST FAILED');
  process.exit(1);
}
