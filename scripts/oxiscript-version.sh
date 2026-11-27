#!/usr/bin/env bash
# OxiScript version tool (rule: docs/oxiscript/VERSIONING.md).
#   scripts/oxiscript-version.sh           print the next version (and the carry for the tag message)
#   scripts/oxiscript-version.sh --check   verify every manifest carries ONE version (exit 1 otherwise)
#
# Rule: x.y.c; c += 1 per 5555 changed lines (added + deleted) in oxiscript/oxiscript-*/src/**
# and the standard library oxiscript/std/**/*.oxs since the last tag `oxiscript-v*`; the remainder
# carries over.
#   scripts/oxiscript-version.sh --owner-release "<reason>"   owner override: next c now, carry 0 The carry lives in the ANNOTATED
# tag message as a line `carry=N`, so the result is deterministic from git alone.
# No tag yet: initial release, prints 0.0.1 (history before the first tag is not counted).
# Exit codes: 0 ok, 1 error / mismatch, 2 usage, 3 no release due yet.
set -eu
cd "$(dirname "$0")/.."
STEP=5555

manifests_check() {
  local versions="" f v
  for f in oxiscript/*/Cargo.toml; do
    v=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$f" | head -n1)
    versions="$versions$f $v
"
  done
  v=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' editors/vscode-oxiscript/package.json | head -n1)
  versions="${versions}editors/vscode-oxiscript/package.json $v
"
  # package-lock.json: the root version appears twice (top level and packages[""]).
  v=$(sed -n '3s/^  "version": "\(.*\)",$/\1/p' editors/vscode-oxiscript/package-lock.json)
  versions="${versions}editors/vscode-oxiscript/package-lock.json(top) $v
"
  v=$(sed -n '/^    "": {$/,/^    },\{0,1\}$/s/^      "version": "\(.*\)",$/\1/p' editors/vscode-oxiscript/package-lock.json | head -n1)
  versions="${versions}editors/vscode-oxiscript/package-lock.json(root) $v
"
  # The playground page reads the CLI manifest at build time (sites/build.mjs): no separate literal.
  local distinct count
  distinct=$(printf '%s' "$versions" | awk 'NF==2{print $2}' | sort -u)
  count=$(printf '%s' "$versions" | awk 'NF!=2 || 1{print $2}' | sort -u | wc -l)
  if [ "$count" -ne 1 ] || [ -z "$distinct" ]; then
    echo "oxiscript version mismatch:" >&2
    printf '%s' "$versions" | awk 'NF{printf "  %-55s %s\n", $1, ($2==""?"(none)":$2)}' >&2
    return 1
  fi
  echo "oxiscript version $distinct: all manifests agree"
}

if [ "${1:-}" = "--check" ]; then manifests_check; exit $?; fi
OWNER=""
if [ "${1:-}" = "--owner-release" ]; then
  [ -n "${2:-}" ] || { echo "usage: $0 --owner-release \"<reason>\"" >&2; exit 2; }
  OWNER="$2"; shift 2
fi
[ $# -eq 0 ] || { echo "usage: $0 [--check | --owner-release \"<reason>\"]" >&2; exit 2; }

TAG=$(git tag -l 'oxiscript-v*' --sort=-v:refname | head -n1)
if [ -z "$TAG" ]; then
  echo "0.0.1"
  echo "initial release (no oxiscript-v* tag yet): tag message must contain carry=0" >&2
  exit 0
fi

# Strict tag name check: oxiscript-vX.Y.C with ASCII decimal digits only (never evaluate arbitrary suffixes).
if [[ ! "$TAG" =~ ^oxiscript-v([0123456789]+)\.([0123456789]+)\.([0123456789]+)$ ]]; then
  echo "invalid tag '$TAG': expected oxiscript-vX.Y.C with decimal digits only" >&2
  exit 1
fi
X=$((10#${BASH_REMATCH[1]}))
Y=$((10#${BASH_REMATCH[2]}))
C=$((10#${BASH_REMATCH[3]}))

# Must be an annotated tag with carry=N (parsed strictly as decimal, never octal).
[ "$(git cat-file -t "refs/tags/$TAG")" = tag ] || { echo "$TAG is a lightweight tag: re-create it annotated with a carry=N line" >&2; exit 1; }
CARRY_RAW=$(git tag -l --format='%(contents)' "$TAG" | sed -n 's/^carry=\([0123456789][0123456789]*\)$/\1/p' | head -n1)
if [ -z "$CARRY_RAW" ] || [[ ! "$CARRY_RAW" =~ ^[0123456789]+$ ]]; then
  echo "$TAG has no 'carry=N' line in its message" >&2
  exit 1
fi
CARRY=$((10#$CARRY_RAW))

# added + deleted lines (same sum as `git diff --shortstat`, binary files skipped).
DIFF=$(git diff --numstat "$TAG" HEAD -- ':(glob)oxiscript/oxiscript-*/src/**' ':(glob)oxiscript/std/**/*.oxs' | awk '$1!="-"{s+=$1+$2} END{print s+0}')
if [[ ! "$DIFF" =~ ^[0123456789]+$ ]]; then
  echo "unexpected diff line count '$DIFF'" >&2
  exit 1
fi
DIFF=$((10#$DIFF))
TOTAL=$((DIFF + CARRY))
BUMP=$((TOTAL / STEP))
NEWCARRY=$((TOTAL % STEP))
echo "since $TAG: $DIFF changed src lines + carry $CARRY = $TOTAL; +$BUMP" >&2
if [ -n "$OWNER" ]; then
  # Owner override (VERSIONING.md): release now even below the step; at least +1; carry resets to 0.
  [ "$BUMP" -ge 1 ] || BUMP=1
  echo "$X.$Y.$((C + BUMP))"
  echo "owner release: tag message must contain: carry=0 and owner-release=$OWNER" >&2
  exit 0
fi
if [ "$BUMP" -eq 0 ]; then
  echo "no release due (need $((STEP - TOTAL)) more changed lines); carry stays $TOTAL" >&2
  exit 3
fi
echo "$X.$Y.$((C + BUMP))"
echo "tag message must contain: carry=$NEWCARRY" >&2
