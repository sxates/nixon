#!/bin/bash
# specs/0065 — file-size gate (supersedes the specs/0042 WS6 per-file ratchet).
#
# Two rules, deliberately separate:
#
#   Rule A (hard cap)  A production source file that is NOT tracked may not
#                      exceed $LIMIT lines. This is what stops new god-files.
#   Rule B (budget)    Across the tracked (grandfathered) files, the TOTAL
#                      excess — sum(max(0, lines - LIMIT)) — may not exceed
#                      the number in scripts/file-size-budget.txt.
#
# Rule B replaces "each allowlisted file may only shrink". Per-file freezing
# made every edit to a legacy file a blocking event regardless of merit, which
# at one point made the code worse: a file sitting at its exact ceiling could
# not take a one-line `use` statement, so a fully-qualified path was written
# instead. A shared budget keeps the downward pressure in aggregate while
# letting a small, justified addition through — and because the budget counts
# excess, it converges to zero exactly when every file is under the cap, so
# this gate retires itself.
#
# Files:
#   scripts/file-size-tracked.txt  paths only, no numbers — the grandfathered set.
#                                  New entries only by deliberate reviewed edit.
#   scripts/file-size-budget.txt   a single integer: the max allowed total excess.
#                                  Lowered by --update; never raised by it.
#
# The budget is the current excess rounded UP to the next $GRID lines, so there is
# always some headroom to absorb an ordinary edit — a budget seeded at exactly
# today's excess would rebuild the same zero-headroom wall this gate exists to
# remove, just one wall instead of 28. The grid also sets what a ratchet step
# means: the budget only drops once you have taken out $GRID lines, i.e. a typical
# module's worth (500 ~ p90 of this repo's production file sizes), rather than
# locking in every stray deleted comment as a permanent new ceiling.
#
# Usage:
#   scripts/check-file-size.sh             # check (CI mode, exit 1 on violation)
#   scripts/check-file-size.sh --update    # drop files now under the cap, lower the budget
#   scripts/check-file-size.sh --self-test # run the gate's own adversarial checks
set -euo pipefail

# FSB_* overrides exist so --self-test can run the real logic against a scratch
# repo. Normal runs never set them.
SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
ROOT=${FSB_ROOT:-"$(cd "$(dirname "$0")/.." && pwd)"}
cd "$ROOT"

LIMIT=${FSB_LIMIT:-800}
GRID=${FSB_GRID:-500}
TRACKED=${FSB_TRACKED:-scripts/file-size-tracked.txt}
BUDGET_FILE=${FSB_BUDGET:-scripts/file-size-budget.txt}

MODE=check
case "${1:-}" in
  "")           MODE=check ;;
  --update)     MODE=update ;;
  --self-test)  MODE=selftest ;;
  *) echo "usage: $0 [--update|--self-test]" >&2; exit 2 ;;
esac

# Production source only: tests, fixtures, and generated bindings are exempt.
files() {
  # NB: not a '**/*.rs' pathspec — git's '**' does not match files at the
  # top of the given directory (lib.rs would be exempt).
  git ls-files frontend/src frontend/src-tauri/src llama-helper/src |
    grep -E '\.(rs|ts|tsx)$' |
    grep -vE '(^|/)(tests?|__tests__|fixtures)/|\.(test|spec)\.tsx?$|^frontend/src/bindings\.ts$'
}

# Tracked list: one path per line; blank lines and # comments ignored.
tracked_paths() {
  [ -f "$TRACKED" ] || return 0
  sed -e 's/[[:space:]]*$//' -e 's/^[[:space:]]*//' "$TRACKED" |
    grep -vE '^(#|$)' || true
}

# Prints the budget, or "bad" if the file is malformed — the caller checks, because
# `exit` inside a $(...) only kills the subshell.
read_budget() {
  [ -f "$BUDGET_FILE" ] || { echo 0; return; }
  local v
  v=$(grep -vE '^[[:space:]]*(#|$)' "$BUDGET_FILE" | head -1 | tr -d '[:space:]')
  case "$v" in
    ''|*[!0-9]*) echo "bad" ;;
    *) echo "$v" ;;
  esac
}

# ---------------------------------------------------------------- self-test --
# Builds a throwaway git repo with the same layout and runs THIS script against
# it via FSB_ROOT, so the checks exercise real discovery, not a reimplementation.
self_test() {
  local sandbox failures=0
  sandbox=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$sandbox'" EXIT

  mk() { # mk <path> <lines>
    mkdir -p "$sandbox/$(dirname "$1")"
    awk -v n="$2" 'BEGIN{for(i=0;i<n;i++) print "// line"}' > "$sandbox/$1"
  }
  run() { # run [args...] -> prints output, returns exit code
    ( cd "$sandbox" && FSB_ROOT="$sandbox" \
        FSB_TRACKED=tracked.txt FSB_BUDGET=budget.txt \
        bash "$SELF" "$@" 2>&1 ) || return $?
  }
  expect() { # expect <want-rc> <label> <args...>
    local want=$1 label=$2; shift 2
    local out rc=0
    out=$(run "$@") || rc=$?
    if [ "$rc" = "$want" ]; then
      echo "  ok   $label"
    else
      echo "  FAIL $label (wanted exit $want, got $rc)"
      echo "$out" | sed 's/^/       | /'
      failures=$((failures + 1))
    fi
  }
  contains() { # contains <label> <needle> <args...>
    local label=$1 needle=$2; shift 2
    local out
    out=$(run "$@" || true)
    if echo "$out" | grep -qF "$needle"; then
      echo "  ok   $label"
    else
      echo "  FAIL $label (output lacked \"$needle\")"
      echo "$out" | sed 's/^/       | /'
      failures=$((failures + 1))
    fi
  }

  mkdir -p "$sandbox/frontend/src" "$sandbox/frontend/src-tauri/src" "$sandbox/llama-helper/src"
  mk frontend/src-tauri/src/big.rs 1200      # tracked, 400 over
  mk frontend/src-tauri/src/small.rs 100     # untracked, fine
  printf 'frontend/src-tauri/src/big.rs\n' > "$sandbox/tracked.txt"
  printf '400\n' > "$sandbox/budget.txt"
  ( cd "$sandbox" && git init -q . && git add -A && git -c user.email=t@t -c user.name=t commit -qm s )

  echo "self-test: clean tree"
  expect 0 "clean tree passes"
  contains "reports excess and budget" "excess 400/400"

  echo "self-test: a one-line addition to a tracked file at the old ceiling"
  mk frontend/src-tauri/src/big.rs 1201
  ( cd "$sandbox" && git add -A )
  printf '401\n' > "$sandbox/budget.txt"   # budget has room
  expect 0 "passes when the budget has room"
  printf '400\n' > "$sandbox/budget.txt"   # budget does not
  expect 1 "fails when it would exceed the budget"
  contains "names the top contributor" "frontend/src-tauri/src/big.rs"

  echo "self-test: Rule A"
  mk frontend/src-tauri/src/big.rs 1200
  mk frontend/src/newgod.tsx 801
  ( cd "$sandbox" && git add -A )
  expect 1 "an 801-line untracked file fails even with budget available"
  contains "Rule A message names the file" "frontend/src/newgod.tsx"
  rm "$sandbox/frontend/src/newgod.tsx"
  ( cd "$sandbox" && git add -A )
  expect 0 "removing it restores green"

  echo "self-test: hygiene"
  printf 'frontend/src-tauri/src/big.rs\nfrontend/src-tauri/src/ghost.rs\n' > "$sandbox/tracked.txt"
  expect 1 "a tracked path that does not exist fails"
  contains "hygiene message names the stale entry" "frontend/src-tauri/src/ghost.rs"
  run --update >/dev/null || true
  expect 0 "--update prunes it"

  echo "self-test: hygiene — a tracked path git knows but the gate exempts"
  # Distinct from "no longer exists": the file is right there in the index, it is
  # simply in a tests/ directory the scan skips, so it can never be pruned by size.
  mk frontend/src-tauri/src/tests/exempt.rs 1500
  ( cd "$sandbox" && git add -A )
  printf 'frontend/src-tauri/src/big.rs\nfrontend/src-tauri/src/tests/exempt.rs\n' > "$sandbox/tracked.txt"
  expect 1 "a tracked file that is no longer scanned fails"
  contains "and distinguishes it from a deleted path" "no longer scanned by this gate"
  run --update >/dev/null || true
  expect 0 "--update prunes it"

  echo "self-test: --update after a decomposition"
  mk frontend/src-tauri/src/big.rs 700       # now under the cap
  ( cd "$sandbox" && git add -A )
  run --update >/dev/null || true
  if grep -q 'big.rs' "$sandbox/tracked.txt"; then
    echo "  FAIL --update drops a file that fell under the cap"; failures=$((failures + 1))
  else
    echo "  ok   --update drops a file that fell under the cap"
  fi
  if [ "$(tr -d '[:space:]' < "$sandbox/budget.txt")" = "0" ]; then
    echo "  ok   --update lowered the budget to 0"
  else
    echo "  FAIL --update should have lowered the budget to 0 (got $(cat "$sandbox/budget.txt"))"
    failures=$((failures + 1))
  fi

  echo "self-test: --update rounds the budget up to the grid, not to the bare excess"
  mk frontend/src-tauri/src/big.rs 1200       # excess 400
  printf 'frontend/src-tauri/src/big.rs\n' > "$sandbox/tracked.txt"
  printf '2000\n' > "$sandbox/budget.txt"
  ( cd "$sandbox" && git add -A )
  run --update >/dev/null || true
  if [ "$(tr -d '[:space:]' < "$sandbox/budget.txt")" = "500" ]; then
    echo "  ok   --update lowered 2000 -> 500 (excess 400 rounded up to the grid)"
  else
    echo "  FAIL --update set the budget to $(cat "$sandbox/budget.txt"); expected 500"
    failures=$((failures + 1))
  fi
  contains "a green run reports its headroom" "100 lines of headroom"

  echo "self-test: --update never raises the budget"
  mk frontend/src-tauri/src/big.rs 1200
  printf 'frontend/src-tauri/src/big.rs\n' > "$sandbox/tracked.txt"
  printf '100\n' > "$sandbox/budget.txt"   # excess 400 already blows this
  ( cd "$sandbox" && git add -A )
  run --update >/dev/null || true
  if [ "$(tr -d '[:space:]' < "$sandbox/budget.txt")" = "100" ]; then
    echo "  ok   --update left the budget alone when excess exceeds it"
  else
    echo "  FAIL --update raised the budget to $(cat "$sandbox/budget.txt"); expected 100"
    failures=$((failures + 1))
  fi
  expect 1 "and the gate still fails until it is paid down"

  echo
  if [ "$failures" -gt 0 ]; then
    echo "file-size gate self-test: $failures failure(s)"
    exit 1
  fi
  echo "file-size gate self-test: ok"
  exit 0
}

if [ "$MODE" = selftest ]; then
  self_test
fi

# -------------------------------------------------------------------- check --
tracked_nl=$'\n'"$(tracked_paths)"$'\n'
is_tracked() {
  case "$tracked_nl" in
    *$'\n'"$1"$'\n'*) return 0 ;;
  esac
  return 1
}

budget=$(read_budget)
if [ "$budget" = bad ]; then
  echo "FAIL: $BUDGET_FILE must contain a single integer (the maximum total excess)" >&2
  exit 1
fi
excess=0
seen=""
contrib=$(mktemp)
over_cap=$(mktemp)
under_cap=$(mktemp)
missing=$(mktemp)
trap 'rm -f "$contrib" "$over_cap" "$under_cap" "$missing" "$seen"' EXIT

seen=$(mktemp)
while IFS= read -r f; do
  lines=$(wc -l < "$f" | tr -d ' ')
  printf '%s\n' "$f" >> "$seen"
  if is_tracked "$f"; then
    if [ "$lines" -gt "$LIMIT" ]; then
      excess=$((excess + lines - LIMIT))
      printf '%s %s %s\n' "$((lines - LIMIT))" "$f" "$lines" >> "$contrib"
    else
      printf '%s %s\n' "$f" "$lines" >> "$under_cap"
    fi
  elif [ "$lines" -gt "$LIMIT" ]; then
    printf '%s %s\n' "$f" "$lines" >> "$over_cap"
  fi
done < <(files)

# Tracked entries the scan never reached: deleted, renamed, or moved somewhere the
# gate exempts (a tests/ directory). Checking against what was actually discovered —
# rather than against `git ls-files` — catches the exempt case too, which would
# otherwise sit in the list forever contributing nothing and never being pruned.
while IFS= read -r p; do
  [ -n "$p" ] || continue
  if ! grep -qxF "$p" "$seen"; then
    if git ls-files --error-unmatch "$p" >/dev/null 2>&1; then
      printf '%s\tis no longer scanned by this gate (moved into an exempt path?)\n' "$p" >> "$missing"
    else
      printf '%s\tno longer exists\n' "$p" >> "$missing"
    fi
  fi
done < <(tracked_paths)

if [ "$MODE" = update ]; then
  changed=0
  while IFS=' ' read -r f lines; do
    [ -n "${f:-}" ] || continue
    sed -i '' "\#^${f}\$#d" "$TRACKED"
    echo "budget: dropped $f (now $lines ≤ $LIMIT)"
    changed=$((changed + 1))
  done < "$under_cap"
  while IFS=$'\t' read -r p why; do
    [ -n "${p:-}" ] || continue
    sed -i '' "\#^${p}\$#d" "$TRACKED"
    echo "budget: dropped $p ($why)"
    changed=$((changed + 1))
  done < "$missing"
  target=$(( (excess + GRID - 1) / GRID * GRID ))
  if [ "$target" -lt "$budget" ]; then
    printf '%s\n' "$target" > "$BUDGET_FILE"
    echo "budget: lowered $budget -> $target (excess $excess, rounded up to the next $GRID)"
    changed=$((changed + 1))
  elif [ "$excess" -gt "$budget" ]; then
    # --update must never paper over a real violation by raising the budget.
    echo "budget: NOT raised — excess $excess exceeds budget $budget; pay it down instead"
  fi
  echo "file-size gate: ok (excess $excess/$budget, $changed change(s))"
  exit 0
fi

violations=0

# Rule A — an untracked file over the cap.
if [ -s "$over_cap" ]; then
  while IFS=' ' read -r f lines; do
    echo "FAIL: $f is $lines lines (limit $LIMIT, and it is not tracked)."
    echo "      Decompose it — or, if this is a deliberate grandfathering decision,"
    echo "      add \"$f\" to $TRACKED and raise $BUDGET_FILE by $((lines - LIMIT)) in the same reviewed diff."
    violations=$((violations + 1))
  done < "$over_cap"
fi

# Rule B — the shared excess budget.
if [ "$excess" -gt "$budget" ]; then
  echo "FAIL: tracked files total $excess lines of excess over the $LIMIT-line cap;"
  echo "      the budget is $budget (over by $((excess - budget)))."
  echo "      Pay it down anywhere in the tracked set — it does not have to be this file."
  echo "      Biggest contributors:"
  sort -rn "$contrib" | head -5 | while IFS=' ' read -r ex f lines; do
    echo "        $f — $lines lines ($ex over)"
  done
  violations=$((violations + 1))
fi

# Hygiene — a tracked path that no longer exists.
if [ -s "$missing" ]; then
  while IFS=$'\t' read -r p why; do
    echo "FAIL: $TRACKED lists $p, which $why — remove the entry"
    echo "      (or run scripts/check-file-size.sh --update)."
    violations=$((violations + 1))
  done < "$missing"
fi

if [ "$violations" -gt 0 ]; then
  echo "file-size gate: $violations violation(s) (excess $excess/$budget)"
  exit 1
fi

if [ -s "$under_cap" ]; then
  while IFS=' ' read -r f lines; do
    echo "note: $f is down to $lines lines — run scripts/check-file-size.sh --update to drop it"
  done < "$under_cap"
fi

echo "file-size gate: ok — excess $excess/$budget across $(wc -l < "$contrib" | tr -d " ") tracked file(s) over the cap ($((budget - excess)) lines of headroom)"
