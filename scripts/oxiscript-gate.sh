#!/usr/bin/env bash
# OxiScript gate. Levels:
#   quick  - every oxiscript crate, debug, one crate at a time (reduced fuzz counts)
#   merge  - quick + one unified run (same Cargo feature unification as CI)
#            + release build with OXI_FULL_FUZZ=1 (full fuzz counts)
#            + the long #[ignore]d fuzz campaigns with fixed seeds
#   nightly- merge, with seeds taken from the date (new inputs every night),
#            plus, with OXI_NIGHTLY_IGNORED=1 (local night run by the lead),
#            every #[ignore]d heavy test in release
# Usage: scripts/oxiscript-gate.sh [quick|merge|nightly]
# Exit code is non-zero if anything fails. A summary is printed at the end.
set -u
LEVEL="${1:-merge}"
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export TMPDIR="${TMPDIR:-$(mktemp -d)}"

CRATES=(oxiscript-syntax oxiscript-abi oxiscript-ir oxiscript-rc oxiscript-runtime
        oxiscript-check oxiscript-wasm oxiscript-lower oxiscript-cli oxiscript-playground)
FAILED=()
step() { # name, command...
  local name="$1"; shift
  local t0=$SECONDS
  if "$@" > "$TMPDIR/gate-$name.log" 2>&1; then
    echo "ok    $name ($((SECONDS - t0)) s)"
  else
    echo "FAIL  $name ($((SECONDS - t0)) s)  -> $TMPDIR/gate-$name.log"
    FAILED+=("$name")
  fi
}

# --- quick
step version-check scripts/oxiscript-version.sh --check
for c in "${CRATES[@]}"; do
  step "test-$c" cargo test -p "$c" --no-fail-fast
done
# Compilation targets: the loops above run the program suites for `device` (the default). Run the
# same suites compiled for `web` and executed on a Web-configured host (OXI_TARGET is read by the
# wasm/lower test helpers); identical results are required (docs/oxiscript/abi.md 3.6).
step targets-web env OXI_TARGET=web cargo test -p oxiscript-wasm -p oxiscript-lower --no-fail-fast
PKGS=(); for c in "${CRATES[@]}"; do PKGS+=(-p "$c"); done
step clippy cargo clippy "${PKGS[@]}" --all-targets -- -D warnings

if [ "$LEVEL" != quick ]; then
  # --- merge: CI-equivalent unified run (catches feature-unification differences)
  step unified cargo test "${PKGS[@]}" --no-fail-fast
  # full fuzz counts (the fuzz tests scale up in release / with OXI_FULL_FUZZ)
  step release-full-fuzz env OXI_FULL_FUZZ=1 cargo test --release --no-fail-fast \
    -p oxiscript-syntax -p oxiscript-rc -p oxiscript-runtime -p oxiscript-check -p oxiscript-wasm -p oxiscript-lower
  # long campaigns
  SEED=1
  if [ "$LEVEL" = nightly ]; then SEED=$(date +%Y%m%d); fi
  step lower-diff-campaign cargo test --release -p oxiscript-lower --test bad_actor_8l_fuzz \
    -- --ignored fuzz_long_seeded
  step parser-roundtrip-oracle env BA_SEED="$SEED" BA_ITERS=3000 BA_NOCOMMENTS=1 BA_NOIMPORT=1 \
    cargo test --release -p oxiscript-syntax --test bad_actor_8p_gen -- --ignored structured_roundtrip
  # Known-red oracles (open findings, tracked; reported, not counted as gate failures):
  #   structured_roundtrip WITH comments/imports -> 8.P formatter findings F1/F2/F4 (comments)
  #   fixit_sweep -> 8.P finding X1 (E0006 no-op fix)
  for o in "structured_roundtrip:BA_SEED=$SEED BA_ITERS=1500" "fixit_sweep:BA_SEED=$SEED BA_ITERS=1500"; do
    name="${o%%:*}"; envs="${o#*:}"
    if env $envs cargo test --release -p oxiscript-syntax --test bad_actor_8p_gen -- --ignored --exact "$name" \
        > "$TMPDIR/gate-known-$name.log" 2>&1; then
      echo "note  known-red oracle $name now PASSES: promote it into the gate"
    else
      echo "known $name still red (open 8.P finding) -> $TMPDIR/gate-known-$name.log"
    fi
  done
fi

if [ "$LEVEL" = nightly ] && [ "${OXI_NIGHTLY_IGNORED:-0}" = 1 ]; then
  # --- nightly only: every #[ignore]d heavy test (large differential campaigns,
  # 100k+ iteration fuzzers, long probes) in release with full fuzz counts.
  # Rule: a test that takes more than a few seconds in debug is #[ignore]d with a
  # reason and runs here, so the day gates stay short.
  for c in "${CRATES[@]}"; do
    step "nightly-ignored-$c" env OXI_FULL_FUZZ=1 BA_SEED="$SEED" cargo test --release -p "$c" --no-fail-fast -- --ignored
  done
fi

echo
if [ ${#FAILED[@]} -eq 0 ]; then
  echo "GATE ($LEVEL): GREEN"
else
  echo "GATE ($LEVEL): RED: ${FAILED[*]}"
  exit 1
fi
