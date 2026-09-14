#!/bin/bash
# specs/0057 Phase B — semantic-token gate.
#
# Fails if any production frontend source uses a raw Tailwind palette color class
# (bg-red-500, text-gray-700, border-white, ring-blue-400, …) or a hex color literal,
# instead of the semantic tokens (bg-background, text-muted-foreground, bg-success/10,
# text-chart-2, hsl(var(--border)), …). Raw palette classes do not flip with `.dark`, so
# every one of them is a light-only bug the moment the Deck theme is on.
#
# Hex detection is deliberately broad: ANY `#rgb`..`#rrggbbaa` literal counts, in a class
# arbitrary value (`bg-[#fff]`), a CSS declaration (`background: #fff`, `box-shadow:
# 0 0 2px #f00`, `linear-gradient(90deg, #fff, #000)`), a React style object
# (`backgroundColor: '#fff'`) or a JSX/SVG attribute (`fill="#fff"`).
#
# Allowed exceptions (add sparingly, with a reason):
#   - a line containing `token-gate: allow` (e.g. a deliberate pure-white overlay)
#   - a CSS custom-property *definition* (`--deck-ink: #101010;`) — that is where a raw
#     value legitimately enters the token system; consumers must use `hsl(var(--…))`.
#   - a comment line (`// …`, `* …`, `/* …`)
# The last two apply to hex literals only; a commented-out palette class still fails,
# because commented-out UI code gets uncommented.
#
# Usage: scripts/check-off-token-colors.sh        # exit 1 on violation
set -euo pipefail
cd "$(dirname "$0")/.."

PALETTE='slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|white|black'
PREFIX='bg|text|border|ring|from|to|via|fill|stroke|shadow|divide|outline|placeholder|decoration|caret|accent'
CLASS_RE="(^|[^A-Za-z0-9_-])(${PREFIX})-(${PALETTE})(-[0-9]{2,3})?([^A-Za-z0-9_-]|$)"
# Any hex color literal. The leading class rejects `&#160;`-style HTML entities and
# longer identifiers; the trailing class keeps `#deadbeef99` from matching as a colour.
HEX_RE='(^|[^0-9A-Za-z_&#-])#[0-9A-Fa-f]{3,8}([^0-9A-Za-z_-]|$)'
# Hex-only exemptions: custom-property definitions and comment lines. The optional `N:`
# prefix tolerates the line number `grep -n` has already prepended.
HEX_EXEMPT='(^|[^A-Za-z0-9_-])--[A-Za-z0-9_-]+[[:space:]]*:|^([0-9]+:)?[[:space:]]*(//|\*|/\*)'

files() {
  git ls-files frontend/src |
    grep -E '\.(ts|tsx|css)$' |
    grep -vE '(^|/)(__tests__|fixtures)/|\.(test|spec)\.tsx?$'
}

# `file:line:match`, one entry per offending line, in file/line order.
listing=$(
  while IFS= read -r f; do
    {
      grep -nE "$CLASS_RE" "$f" || true
      grep -nE "$HEX_RE" "$f" | grep -vE "$HEX_EXEMPT" || true
    } | grep -v 'token-gate: allow' | sed "s|^|$f:|" || true
  done < <(files) | sort -t: -k1,1 -k2,2n -u
)

if [ -n "$listing" ]; then
  printf '%s\n' "$listing"
  violations=$(printf '%s\n' "$listing" | wc -l | tr -d '[:space:]')
  echo >&2
  echo "check-off-token-colors: $violations raw palette/hex color usage(s). Use semantic tokens" >&2
  echo "(see specs/0057 §Decisions + frontend/tailwind.config.js colors)." >&2
  exit 1
fi
echo "check-off-token-colors: ok"
