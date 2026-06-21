#!/bin/sh
# Verify the version is identical across every manifest that carries one.
#
# surd's version lives in five files (three Cargo crates, the Tauri config, and
# the npm package). They must agree so the CLI banner, the wasm engine, the app
# UI, and the release tag all report the same number. This script is the guard:
# it's run by the pre-commit hook and as the first job of the release workflow,
# and exits non-zero (printing every value) the moment they drift.
#
# Pure POSIX sh + awk so it runs on CI before any toolchain is installed. Bump
# all five at once with scripts/bump-version.sh.
set -eu

cd "$(git rev-parse --show-toplevel)"

# Version inside a Cargo.toml's [package] table (skips dependency `version =`).
cargo_version() {
  awk '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/          { in_pkg = 0 }
    in_pkg && /^version[[:space:]]*=/ {
      gsub(/.*=[[:space:]]*"/, ""); gsub(/".*/, ""); print; exit
    }
  ' "$1"
}

# First top-level "version": "..." in a JSON file.
json_version() {
  awk '
    /"version"[[:space:]]*:/ {
      gsub(/.*"version"[[:space:]]*:[[:space:]]*"/, ""); gsub(/".*/, ""); print; exit
    }
  ' "$1"
}

# label  file  extracted-version  — collected for a single aligned report.
set -- \
  "Cargo.toml (surd)               $(cargo_version Cargo.toml)" \
  "wasm/Cargo.toml (surd-wasm)     $(cargo_version wasm/Cargo.toml)" \
  "app/src-tauri/Cargo.toml        $(cargo_version app/src-tauri/Cargo.toml)" \
  "app/src-tauri/tauri.conf.json   $(json_version app/src-tauri/tauri.conf.json)" \
  "app/package.json                $(json_version app/package.json)"

ref="$(cargo_version Cargo.toml)"
ok=1
for line in "$@"; do
  ver="${line##* }"
  printf '  %s\n' "$line"
  [ "$ver" = "$ref" ] || ok=0
done

if [ "$ok" -ne 1 ]; then
  echo "error: version mismatch — all five must equal '$ref'." >&2
  echo "       run: scripts/bump-version.sh <version>" >&2
  exit 1
fi

echo "version OK: $ref"
