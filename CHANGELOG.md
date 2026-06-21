# Changelog

All notable changes to surd are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under `[Unreleased]`; `scripts/bump-version.sh` rolls that
section into a dated, versioned release.

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/dariusjlukas/surd/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/dariusjlukas/surd/releases/tag/v0.1.0
