# Changelog

All notable changes to surd are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under `[Unreleased]`; `scripts/bump-version.sh` rolls that
section into a dated, versioned release.

## [Unreleased]

## [0.5.0] - 2026-06-23

## [0.4.0] - 2026-06-22

## [0.3.0] - 2026-06-22

### Added

- 3D scatter plots: `scatter3d(x, y, z)` draws three equal-length vectors as
  markers in the surface view. Overlay it in `plot3d(...)` to compare a fitted
  surface against measured `(x, y, z)` data —
  e.g. `plot3d(b0 + b1*x + b2*y, scatter3d(xs, ys, zs), x, 0, 10, y, 0, 10)` —
  or `plot3d(scatter3d(xs, ys, zs))` on its own, auto-boxed to the data. Points
  are static (orbit/zoom without resampling) and the hover probe snaps to the
  nearest marker.

## [0.2.0] - 2026-06-21

### Added

- Scatter plots: `scatter(x, y)` draws two equal-length vectors as markers.
  Overlay it in `plot(...)` to compare measured data against a curve —
  e.g. `plot(scatter(xs, ys), m.predict, x, a, b)` — or `plot(scatter(xs, ys))`
  on its own, auto-windowed to the data. Points are static (pan/zoom re-windows
  client-side) and the hover probe snaps to the nearest point.
- Fitted models are directly plottable: `stats.linfit` and `stats.nlfit` now
  return a `predict` field holding the fitted curve as a function — evaluate it
  (`m.predict(2.5)`) or plot it (`plot(scatter(xs, ys), m.predict, x, a, b)`).
  Relatedly, `plot` now accepts any one-argument function as a curve, so
  `plot(f, x, a, b)` draws a user-defined `f` directly.
- Offline documentation in the desktop app: the Help button now opens a copy
  of the docs bundled into the build (in its own window, no network), falling
  back to the hosted site only when a build shipped without them. The web build
  still links to the hosted docs.
- Version reporting: `surd --version` on the CLI, a `version()` binding in the
  wasm engine, and a version line (with a link to the releases page) in the
  desktop/web app's Settings → About.

## [0.1.0]

First tagged release. Exact-by-default computer-algebra engine with:

- Exact arithmetic over arbitrary-precision rationals, radicals, and symbolic
  constants; floats only on explicit `N(x)`.
- A REPL/CLI front end and a wasm-powered notebook UI (web + Tauri desktop).
- Numerical tooling: GLMs, penalized/weighted regression, nonlinear least
  squares with an exact symbolic Jacobian, plus DSP and statistics namespaces.

[Unreleased]: https://github.com/dariusjlukas/surd/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.5.0
[0.4.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.4.0
[0.3.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.3.0
[0.2.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.2.0
[0.1.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.1.0
