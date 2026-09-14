#!/bin/bash
# specs/0042 WS6 — file-size ratchet.
#
# Fails if any production source file (.rs/.ts/.tsx) exceeds $LIMIT lines,
# unless it's listed in scripts/file-size-allowlist.txt — and allowlisted
# files may only shrink: growing past the recorded size also fails.
# The allowlist is a ratchet: entries are removed/tightened as files get
# decomposed (run with --update to do that automatically); new entries are
# added only by deliberate reviewed edit.
#
# Usage:
#   scripts/check-file-size.sh            # check (CI mode, exit 1 on violation)
#   scripts/check-file-size.sh --update   # tighten/remove allowlist entries that shrank
set -euo pipefail
cd "$(dirname "$0")/.."

LIMIT=800
ALLOWLIST=scripts/file-size-allowlist.txt
UPDATE=0
[ "${1:-}" = "--update" ] && UPDATE=1

# Production source only: tests, fixtures, and generated bindings are exempt.
files() {
  # NB: not a '**/*.rs' pathspec — git's '**' does not match files at the
  # top of the given directory (lib.rs would be exempt).
  git ls-files frontend/src frontend/src-tauri/src llama-helper/src |
    grep -E '\.(rs|ts|tsx)$' |
    grep -vE '(^|/)(tests?|__tests__|fixtures)/|\.(test|spec)\.tsx?$|^frontend/src/bindings\.ts$'
}

# allowlist lines: "<max-lines> <path>" (comments and blanks ignored)
allowed_max() {
  [ -f "$ALLOWLIST" ] || { echo ""; return; }
  awk -v f="$1" '$0 !~ /^[[:space:]]*(#|$)/ && $2 == f {print $1; exit}' "$ALLOWLIST"
}

violations=0
tightened=0
while IFS= read -r f; do
  lines=$(wc -l < "$f" | tr -d ' ')
  max=$(allowed_max "$f")
  if [ -n "$max" ]; then
    if [ "$lines" -gt "$max" ]; then
      echo "FAIL: $f is $lines lines (allowlisted at $max — allowlisted files may only shrink)"
      violations=$((violations + 1))
    elif [ "$lines" -lt "$max" ]; then
      if [ "$UPDATE" = 1 ]; then
        if [ "$lines" -le "$LIMIT" ]; then
          sed -i '' "\#^[0-9]* $f\$#d" "$ALLOWLIST"
          echo "ratchet: removed $f (now $lines ≤ $LIMIT)"
        else
          sed -i '' "s#^[0-9]* $f\$#$lines $f#" "$ALLOWLIST"
          echo "ratchet: $f tightened $max -> $lines"
        fi
        tightened=$((tightened + 1))
      else
        echo "note: $f shrank ($lines < allowlisted $max) — run scripts/check-file-size.sh --update to ratchet"
      fi
    fi
  elif [ "$lines" -gt "$LIMIT" ]; then
    echo "FAIL: $f is $lines lines (limit $LIMIT). Decompose it, or (with reviewer sign-off)"
    echo "      add \"$lines $f\" to $ALLOWLIST"
    violations=$((violations + 1))
  fi
done < <(files)

# prune allowlist entries whose file no longer exists (deleted/renamed)
if [ -f "$ALLOWLIST" ]; then
  while read -r max f; do
    if ! git ls-files --error-unmatch "$f" >/dev/null 2>&1; then
      if [ "$UPDATE" = 1 ]; then
        sed -i '' "\#^[0-9]* $f\$#d" "$ALLOWLIST"
        echo "ratchet: removed $f (file no longer exists)"
        tightened=$((tightened + 1))
      else
        echo "note: allowlisted $f no longer exists — run --update to prune"
      fi
    fi
  done < <(awk '$0 !~ /^[[:space:]]*(#|$)/' "$ALLOWLIST")
fi

if [ "$violations" -gt 0 ]; then
  echo "file-size ratchet: $violations violation(s)"
  exit 1
fi
[ "$UPDATE" = 1 ] && echo "file-size ratchet: ok ($tightened entr(y/ies) tightened)" || \
  echo "file-size ratchet: ok"
