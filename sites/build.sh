#!/usr/bin/env bash
# Build the three static sites into sites/dist/{home,oxiui,oxiscript} (slice 8.W).
#   sites/build.sh             full: wasm (release) + pages + checks + cargo test of the playground
#   sites/build.sh --pages     pages only (reuses sites/.build/pkg_*), then checks
#   sites/build.sh --preview   full build, then serve :8081 (home) :8082 (oxiui) :8083 (oxiscript)
# Environment:
#   SITES_LANGS=pl,en          published languages (default). Latin after the owner's review: SITES_LANGS=pl,en,la
#   CARGO_BUILD_JOBS=2         default; never run two builds at once on the old machine
# Needs: rust + wasm32-unknown-unknown, wasm-pack (fetches wasm-opt itself), node >= 18, python3 (preview only).
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
MODE="${1:-full}"
PKG="$PWD/sites/.build"
mkdir -p "$PKG"

if [[ "$MODE" != "--pages" ]]; then
  # 1. OxiUI local-mode bundle (7.1): the core + painter + fonts, running in the browser.
  wasm-pack build --target web --release --out-dir "$PKG/pkg_local" --out-name oxiterm_local \
    oxiterm-web -- --features local
  # 2. OxiScript playground: checker + formatter. Size-optimised per invocation (the workspace has no
  # wasm release profile and the root Cargo.toml is not ours to edit).
  CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_LTO=true \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort \
    wasm-pack build --target web --release --out-dir "$PKG/pkg_play" --out-name oxiscript_playground \
    oxiscript/oxiscript-playground
fi

export OXI_LOCAL_PKG="$PKG/pkg_local" OXI_PLAY_PKG="$PKG/pkg_play"
node sites/build.mjs
node sites/check.mjs

if [[ "$MODE" != "--pages" ]]; then
  # every OxiScript example on the pages passes `check` and is canonically formatted
  cargo test -p oxiscript-playground
fi

echo
echo "Sizes (raw bytes):"
for s in home oxiui oxiscript; do printf '  %-10s %s\n' "$s" "$(du -sb "sites/dist/$s" | cut -f1)"; done
ls -l sites/dist/oxiui/app/*/*.wasm sites/dist/oxiscript/play/*/*.wasm | awk '{print "  " $5 "  " $9}'

if [[ "$MODE" == "--preview" ]]; then
  echo "Preview: http://localhost:8081 (home)  http://localhost:8082 (oxiui)  http://localhost:8083 (oxiscript)"
  echo "(links between the sites go to the real domains unless ORIGIN_HOME / ORIGIN_OXIUI / ORIGIN_OXISCRIPT were set)"
  python3 -m http.server 8081 -d sites/dist/home >/dev/null 2>&1 &
  python3 -m http.server 8082 -d sites/dist/oxiui >/dev/null 2>&1 &
  python3 -m http.server 8083 -d sites/dist/oxiscript >/dev/null 2>&1 &
  trap 'kill $(jobs -p) 2>/dev/null' EXIT
  wait
fi
