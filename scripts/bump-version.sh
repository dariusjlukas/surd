#!/usr/bin/env bash
# Set surd's version everywhere at once and roll the changelog.
#
# Usage:
#   scripts/bump-version.sh <version> [--tag]
#
#   <version>   semver, no leading 'v'   e.g. 0.2.0   or   1.0.0-rc.1
#   --tag       also create the release commit + annotated tag (not pushed)
#
# Touches the five version fields (kept in lockstep by check-version.sh), the
# two Cargo.lock files, and CHANGELOG.md (promotes [Unreleased] to the new
# dated version). Without --tag it edits files and prints the git commands to
# run; with --tag it commits and tags so you only have to:
#   git push --follow-tags
# which fires .github/workflows/release.yml: it builds the macOS/Windows/Linux
# desktop installers and publishes them as a GitHub Release for the tag.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

NEW="${1:-}"
TAG=0
[ "${2:-}" = "--tag" ] && TAG=1

if [ -z "$NEW" ]; then
  echo "usage: scripts/bump-version.sh <version> [--tag]" >&2
  exit 2
fi
if ! printf '%s' "$NEW" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
  echo "error: '$NEW' is not a semver version (expected e.g. 0.2.0)." >&2
  exit 2
fi

# --- editors (awk to a temp file, then move — portable across mac/linux) ----

set_cargo_version() { # file version  — the [package] version only
  local file="$1" ver="$2" tmp
  tmp="$(mktemp)"
  awk -v ver="$ver" '
    /^\[package\]/                 { in_pkg = 1 }
    /^\[/ && !/^\[package\]/        { in_pkg = 0 }
    in_pkg && !done && /^version[[:space:]]*=/ {
      print "version = \"" ver "\""; done = 1; next
    }
    { print }
  ' "$file" >"$tmp" && mv "$tmp" "$file"
}

set_json_version() { # file version  — the first top-level "version"
  local file="$1" ver="$2" tmp
  tmp="$(mktemp)"
  awk -v ver="$ver" '
    !done && /"version"[[:space:]]*:/ {
      sub(/"version"[[:space:]]*:[[:space:]]*"[^"]*"/, "\"version\": \"" ver "\""); done = 1
    }
    { print }
  ' "$file" >"$tmp" && mv "$tmp" "$file"
}

set_lock_version() { # file crate version  — the [[package]] block named <crate>
  local file="$1" crate="$2" ver="$3" tmp
  [ -f "$file" ] || return 0
  tmp="$(mktemp)"
  awk -v crate="$crate" -v ver="$ver" '
    /^\[\[package\]\]/                      { hit = 0 }
    $0 == "name = \"" crate "\""            { hit = 1 }
    hit && /^version[[:space:]]*=/          { print "version = \"" ver "\""; hit = 0; next }
    { print }
  ' "$file" >"$tmp" && mv "$tmp" "$file"
}

update_changelog() { # version date
  local ver="$1" date="$2" tmp
  [ -f CHANGELOG.md ] || { echo "note: no CHANGELOG.md, skipping"; return 0; }
  tmp="$(mktemp)"
  awk -v ver="$ver" -v date="$date" '
    /^## \[Unreleased\]$/ && !promoted {
      print "## [Unreleased]"; print ""; print "## [" ver "] - " date
      promoted = 1; next
    }
    /^\[Unreleased\]:/ {
      print "[Unreleased]: https://github.com/dariusjlukas/surd/compare/v" ver "...HEAD"
      print "[" ver "]: https://github.com/dariusjlukas/surd/releases/tag/v" ver
      next
    }
    { print }
  ' CHANGELOG.md >"$tmp" && mv "$tmp" CHANGELOG.md
}

# --- apply ------------------------------------------------------------------

set_cargo_version Cargo.toml "$NEW"
set_cargo_version wasm/Cargo.toml "$NEW"
set_cargo_version app/src-tauri/Cargo.toml "$NEW"
set_json_version app/src-tauri/tauri.conf.json "$NEW"
set_json_version app/package.json "$NEW"

set_lock_version Cargo.lock surd "$NEW"
set_lock_version Cargo.lock surd-wasm "$NEW"
set_lock_version app/src-tauri/Cargo.lock surd-desktop "$NEW"

update_changelog "$NEW" "$(date +%F)"

# Confirm everything agrees before handing back.
scripts/check-version.sh

echo
echo "Bumped surd to v$NEW."

if [ "$TAG" -eq 1 ]; then
  git add -A
  git commit -m "Release v$NEW"
  git tag -a "v$NEW" -m "surd v$NEW"
  echo
  echo "Committed and tagged v$NEW. Push to build + publish the release:"
  echo "  git push --follow-tags"
else
  echo
  echo "Review the diff, then:"
  echo "  git add -A && git commit -m \"Release v$NEW\""
  echo "  git tag -a \"v$NEW\" -m \"surd v$NEW\""
  echo "  git push --follow-tags"
fi
