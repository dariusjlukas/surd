#!/usr/bin/env node
// Build a self-contained, offline copy of the docs into the desktop app's
// bundle (app/dist/docs), so the in-app Help button works with no network.
//
// Mirrors the GitHub Pages docs build (deploy-pages.yml) but via
// mkdocs.offline.yml (use_directory_urls off; see that file). Run after the
// Vite bundle — Vite empties app/dist on build, so the docs must land after.
// Wired into Tauri's beforeBuildCommand through the `build:offline-docs`
// npm script.
//
// If MkDocs isn't installed this warns and skips (exit 0) rather than failing,
// so a local `tauri build` doesn't hard-require Python tooling — the app falls
// back to the hosted docs at runtime. Release CI installs mkdocs-material, so
// official builds always ship the docs.
//
// Node (not a shell script) so paths and the launcher lookup work the same on
// macOS, Linux, and the Windows release runner.

import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..')
const config = join(repoRoot, 'mkdocs.offline.yml')
const out = join(repoRoot, 'app', 'dist', 'docs')

// Prefer the `mkdocs` launcher; fall back to `python -m mkdocs` (the launcher
// isn't always on PATH, especially on Windows). The first invocation that
// answers `--version` is the one we build with.
function resolveMkdocs() {
  const candidates = [['mkdocs'], ['python3', '-m', 'mkdocs'], ['python', '-m', 'mkdocs']]
  for (const cmd of candidates) {
    const probe = spawnSync(cmd[0], [...cmd.slice(1), '--version'], { stdio: 'ignore' })
    if (probe.status === 0) return cmd
  }
  return null
}

const mkdocs = resolveMkdocs()
if (!mkdocs) {
  console.warn(
    'warning: mkdocs not found — offline docs will NOT be bundled into the app.\n' +
      '         install it with: pip install mkdocs-material\n' +
      '         (without it the desktop app falls back to the hosted docs)',
  )
  process.exit(0)
}

console.log(`Building offline docs -> ${out}`)
const build = spawnSync(
  mkdocs[0],
  [...mkdocs.slice(1), 'build', '--strict', '-f', config, '-d', out],
  { stdio: 'inherit' },
)
process.exit(build.status ?? 1)
