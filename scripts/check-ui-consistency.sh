#!/bin/bash
# ADR-0019 — UI-consistency gate (docs/design-system.md is the contract).
#
# Counts violations of the mechanically checkable design-system rules in production
# frontend source and holds each rule to a budget that only goes down — the same ratchet
# as the file-size gate (specs/0065). Existing drift is grandfathered as today's count;
# new drift fails, and every migration wave lowers the budget with --update.
#
# Rules (one count per OFFENDING OCCURRENCE — two hits on one line count twice; the one
# exception is label-override, which is a per-line rule and counts once per line):
#
#   raw-button           JSX `<button` (incl. a multi-line `<button` that ends its line)
#   raw-input            JSX `<input` / `<select` / `<textarea`
#   arbitrary-text-size  `text-[13px]`, `text-[0.8rem]` — use the named type scale
#   arbitrary-radius     `rounded-[…]` (any side), `rounded-xl|2xl|3xl` (any side), and the
#                        bare `rounded` / `rounded-t|tl|…` class (Tailwind's 4px alias for
#                        rounded-lg). `rounded-sm|md|lg|full|none` are not matched.
#   label-override       `u-section-label` and `text-[` or `tracking-[` on the SAME line
#                        (an approximation of "the same class string"; multi-line `cn(…)`
#                        calls that split them are not seen)
#   arbitrary-shadow     `shadow-[…]` — use shadow-card or the primitive's shadow
#   lucide-import        `from 'lucide-react'` — import glyphs from @/components/ui/icons
#   motion-travel        framer-motion `initial={{ … y: … }}` / `x:` entrance offsets
#                        (same-line approximation: a multi-line initial object is not seen)
#   handrolled-hover     `hover:bg-brand/90`-style hover tints (brand, destructive, primary, record, success) — use the primitive's variant
#
# Scope: git-tracked `.ts` / `.tsx` / `.css` under frontend/src, EXCLUDING
# components/ui/ (the primitives themselves), __tests__/, fixtures/, *.test.* / *.spec.*,
# and app/globals.css (where the tokens and text-role utilities are defined).
#
# Not counted:
#   - comment lines: a line starting with `//` or `*`, and block comments that open at the
#     start of a line (`/* …`, JSX `{/* …`) through their `*/`. Prose that mentions
#     `<button>` is not UI. A `/*` mid-line is NOT treated as a comment opener, because
#     `accept="audio/*"`-style strings would otherwise swallow the rest of the file.
#   - a line containing `ui-gate: allow` (write the reason after it).
#   - files/rules listed in scripts/ui-consistency-allow.txt:
#       path<TAB>rule<TAB>reason        (path = a file, or a directory ending in `/`)
#     Allowlisting is per rule: an entry for arbitrary-shadow exempts nothing else.
#
# Budget: scripts/ui-consistency-budget.txt, one `rule=count` per line (# comments ok).
# A rule missing from it has budget 0. --update lowers each budget to the current count
# and never raises one.
#
# Usage:
#   scripts/check-ui-consistency.sh               # check (CI mode, exit 1 on violation)
#   scripts/check-ui-consistency.sh --update      # lower budgets to today's counts
#   scripts/check-ui-consistency.sh --list <rule> # print every hit for one rule
#   scripts/check-ui-consistency.sh --self-test   # run the gate's own adversarial checks
set -euo pipefail

# UIC_* overrides exist so --self-test can run the real logic against a scratch repo.
# Normal runs never set them.
SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
ROOT=${UIC_ROOT:-"$(cd "$(dirname "$0")/.." && pwd)"}
cd "$ROOT"

BUDGET_FILE=${UIC_BUDGET:-scripts/ui-consistency-budget.txt}
ALLOW_FILE=${UIC_ALLOW:-scripts/ui-consistency-allow.txt}

usage() { echo "usage: $0 [--update | --list <rule> | --self-test]" >&2; exit 2; }

MODE=check
LIST_RULE=""
case "${1:-}" in
  "")          MODE=check ;;
  --update)    MODE=update ;;
  --self-test) MODE=selftest ;;
  --list)      MODE=list; LIST_RULE=${2:-}; [ -n "$LIST_RULE" ] || usage ;;
  *) usage ;;
esac

# rule<TAB>ERE. Matched with awk's match() (POSIX ERE, no interval braces — mawk on the
# ubuntu CI runner and BSD awk on macOS both have to accept them). The regex for
# label-override is a placeholder; that rule is evaluated specially (see the awk below).
RULES=$(cat <<'EOF'
raw-button	<button([[:space:]>/]|$)
raw-input	<(input|select|textarea)([[:space:]>/]|$)
arbitrary-text-size	text-\[[0-9]*\.?[0-9]+(px|rem)\]
arbitrary-radius	(^|[^A-Za-z0-9_-])rounded(-[trbl][trbl]?)?(-\[|-(xl|2xl|3xl)([^A-Za-z0-9_-]|$)|([^A-Za-z0-9_[-]|$))
label-override	u-section-label
arbitrary-shadow	shadow-\[
lucide-import	from[[:space:]]*['"]lucide-react(/[^'"]*)?['"]
motion-travel	initial=\{\{([^}]*[[:space:],])?[xy][[:space:]]*:
handrolled-hover	hover:bg-(brand|destructive|primary|record|success)/[0-9]+
EOF
)
RULE_NAMES=$(printf '%s\n' "$RULES" | cut -f1)

is_rule() { printf '%s\n' "$RULE_NAMES" | grep -qxF "$1"; }

files() {
  git ls-files frontend/src |
    grep -E '\.(ts|tsx|css)$' |
    grep -vE '^frontend/src/components/ui/|(^|/)(__tests__|fixtures)/|\.(test|spec)\.[a-z]+$|^frontend/src/app/globals\.css$' || true
}

# Prints the budget for a rule (0 when absent), or "bad" when the entry is malformed.
budget_for() {
  [ -f "$BUDGET_FILE" ] || { echo 0; return; }
  local v
  v=$(grep -E "^[[:space:]]*$1[[:space:]]*=" "$BUDGET_FILE" | head -1 | cut -d= -f2 | tr -d '[:space:]' || true)
  case "$v" in
    '') echo 0 ;;
    *[!0-9]*) echo bad ;;
    *) echo "$v" ;;
  esac
}

# Allowlist entries, comments/blank lines dropped.
allow_entries() {
  [ -f "$ALLOW_FILE" ] || return 0
  grep -vE '^[[:space:]]*(#|$)' "$ALLOW_FILE" || true
}

# Emits `rule<TAB>file:line<TAB>text` for every hit, plus `ALLOWUSED<TAB>n` for each
# allowlist entry (by index) that exempted at least one hit.
scan() {
  local rules_tmp allow_tmp list_tmp
  rules_tmp=$(mktemp); allow_tmp=$(mktemp); list_tmp=$(mktemp)
  printf '%s\n' "$RULES" > "$rules_tmp"
  allow_entries > "$allow_tmp"
  files > "$list_tmp"
  if [ -s "$list_tmp" ]; then
    # xargs batches the file list; awk state resets per file (FNR==1).
    tr '\n' '\0' < "$list_tmp" | xargs -0 awk -v RULESF="$rules_tmp" -v ALLOWF="$allow_tmp" '
      BEGIN {
        while ((getline l < RULESF) > 0) {
          t = index(l, "\t"); n++; name[n] = substr(l, 1, t - 1); re[n] = substr(l, t + 1)
        }
        while ((getline l < ALLOWF) > 0) {
          split(l, a, "\t"); na++; ap[na] = a[1]; ar[na] = a[2]
        }
      }
      FNR == 1 {
        inblock = 0
        for (i = 1; i <= n; i++) {
          allowed[i] = 0
          for (j = 1; j <= na; j++) {
            if (ar[j] != name[i]) continue
            if (FILENAME == ap[j] || (substr(ap[j], length(ap[j])) == "/" && index(FILENAME, ap[j]) == 1)) {
              allowed[i] = j
            }
          }
        }
      }
      {
        line = $0
        if (inblock) {
          p = index(line, "*/"); if (!p) next
          line = substr(line, p + 2); inblock = 0
        }
        if (line ~ /^[[:space:]]*\{?\/\*/) {
          p = index(line, "*/")
          if (!p) { inblock = 1; next }
          line = substr(line, p + 2)
        }
        if (line ~ /^[[:space:]]*(\/\/|\*)/) next
        if (index(line, "ui-gate: allow")) next
        for (i = 1; i <= n; i++) {
          if (name[i] == "label-override") {
            hit = index(line, "u-section-label") && (index(line, "text-[") || index(line, "tracking-["))
            if (hit) {
              if (allowed[i]) used[allowed[i]] = 1
              else print name[i] "\t" FILENAME ":" FNR "\t" $0
            }
            continue
          }
          s = line
          while (match(s, re[i])) {
            if (allowed[i]) used[allowed[i]] = 1
            else print name[i] "\t" FILENAME ":" FNR "\t" $0
            # Resume ON the last matched character: it may be the trailing boundary of
            # this hit and the leading boundary of the next ("rounded rounded-xl").
            adv = RSTART + RLENGTH - 1
            if (RLENGTH < 2) adv = RSTART + 1
            s = substr(s, adv)
          }
        }
      }
      END { for (j in used) print "ALLOWUSED\t" j }
    ' || true
  fi
  rm -f "$rules_tmp" "$allow_tmp" "$list_tmp"
}

# ---------------------------------------------------------------- self-test --
# Builds a throwaway git repo with the same layout and runs THIS script against it via
# UIC_ROOT, so the checks exercise real discovery and matching, not a reimplementation.
self_test() {
  local sandbox failures=0
  sandbox=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$sandbox'" EXIT

  put() { # put <path> <content>
    mkdir -p "$sandbox/$(dirname "$1")"
    printf '%s\n' "$2" > "$sandbox/$1"
    ( cd "$sandbox" && git add -A )
  }
  run() {
    ( cd "$sandbox" && UIC_ROOT="$sandbox" UIC_BUDGET=budget.txt UIC_ALLOW=allow.txt \
        bash "$SELF" "$@" 2>&1 ) || return $?
  }
  ok()   { echo "  ok   $1"; }
  fail() { echo "  FAIL $1"; failures=$((failures + 1)); }
  # hits <rule> -> number of hits --list reports
  hits() { run --list "$1" | grep -c '^frontend/' || true; }
  expect_hits() { # expect_hits <rule> <want> <label>
    local got; got=$(hits "$1")
    if [ "$got" = "$2" ]; then ok "$3"; else fail "$3 (rule $1: wanted $2 hit(s), got $got)"; { run --list "$1" || true; } | sed 's/^/       | /'; fi
  }
  expect_rc() { # expect_rc <want> <label> <args...>
    local want=$1 label=$2 out rc=0; shift 2
    out=$(run "$@") || rc=$?
    if [ "$rc" = "$want" ]; then ok "$label"; else fail "$label (wanted exit $want, got $rc)"; echo "$out" | sed 's/^/       | /'; fi
  }
  budget_is() { # budget_is <rule> <want> <label>
    local got; got=$(grep -E "^$1=" "$sandbox/budget.txt" | cut -d= -f2)
    if [ "$got" = "$2" ]; then ok "$3"; else fail "$3 (budget for $1: wanted $2, got ${got:-<none>})"; fi
  }

  ( cd "$sandbox" && git init -q . )
  : > "$sandbox/budget.txt"
  : > "$sandbox/allow.txt"
  put frontend/src/app/page.tsx "export const x = 1;"
  ( cd "$sandbox" && git -c user.email=t@t -c user.name=t commit -qm s )

  echo "self-test: an empty tree passes with an empty budget"
  expect_rc 0 "clean tree passes"

  echo "self-test: each rule detects a planted violation"
  put frontend/src/p/RawButton.tsx '  return <button onClick={go}>Go</button>;'
  expect_hits raw-button 1 "raw-button: <button …>"
  put frontend/src/p/RawButtonMulti.tsx $'  return (\n    <button\n      type="button"\n    />\n  );'
  expect_hits raw-button 2 "raw-button: a multi-line <button that ends its line"
  put frontend/src/p/RawInput.tsx $'<input value={v} />\n<select>\n<textarea/>'
  expect_hits raw-input 3 "raw-input: <input, <select, <textarea"
  put frontend/src/p/Text.tsx "<span className=\"text-[13px] md:text-[0.8rem]\" />"
  expect_hits arbitrary-text-size 2 "arbitrary-text-size: px and rem, two on one line"
  put frontend/src/p/Radius.tsx "<div className=\"rounded rounded-[3px] rounded-t-[2px] rounded-xl rounded-b-2xl rounded-3xl rounded-tl\" />"
  expect_hits arbitrary-radius 7 "arbitrary-radius: bare, [..], sided, xl/2xl/3xl, adjacent hits"
  put frontend/src/p/Label.tsx "<span className=\"u-section-label text-[9px]\" /><i className=\"u-section-label tracking-[0.2em]\" />"
  expect_hits label-override 1 "label-override: one per line"
  put frontend/src/p/Shadow.tsx "<div className=\"shadow-[0_0_4px_red]\" />"
  expect_hits arbitrary-shadow 1 "arbitrary-shadow: shadow-[…]"
  put frontend/src/p/Lucide.tsx $'import { X } from \'lucide-react\';\nimport { Y } from "lucide-react";'
  expect_hits lucide-import 2 "lucide-import: single and double quotes"
  put frontend/src/p/Motion.tsx $'<motion.div initial={{ opacity: 0, y: 8 }} />\n<motion.div initial={{x: -4}} />'
  expect_hits motion-travel 2 "motion-travel: y: and x: entrance offsets"
  put frontend/src/p/Hover.tsx "<b className=\"hover:bg-brand/90 hover:bg-destructive/80 hover:bg-primary/5\" />"
  expect_hits handrolled-hover 3 "handrolled-hover: brand, destructive, primary"
  put frontend/src/p/Styles.css ".x { @apply rounded-2xl; }"
  expect_hits arbitrary-radius 8 "a .css file is scanned"

  echo "self-test: near misses are not counted"
  put frontend/src/p/Clean.tsx "$(cat <<'NEAR'
const wellrounded = 1; const notrounded_xl = 2;
import { Button } from '@/components/ui/button';
import type { LucideIcon } from '@/components/ui/icons';
<Button className="rounded-sm rounded-md rounded-lg rounded-full rounded-none rounded-tl-md rounded-b-lg text-xs text-micro shadow-card hover:bg-muted hover:bg-brand-wash" />
<motion.div initial={{ opacity: 0 }} animate={{ y: 0 }} />
<span className="u-section-label" />
<span className="text-[9px]" />
const roundedValue = Math.round(x); type T = HTMLButtonElement | HTMLInputElement;
<Input /><Select /><Textarea />
// a <button> in a line comment
/* a <button> in a block comment opener
   still a comment: <select>, rounded-xl, text-[9px]
*/
{/* JSX comment: <input> */}
<a href="audio/*">x</a>
NEAR
)"
  local stray
  stray=$(run --list raw-button; run --list raw-input; run --list label-override; run --list motion-travel; run --list handrolled-hover; run --list lucide-import; run --list arbitrary-shadow)
  if echo "$stray" | grep -q 'Clean.tsx'; then fail "near misses flagged:"; echo "$stray" | grep 'Clean.tsx' | sed 's/^/       | /'; else ok "no rule flags the near-miss file"; fi
  expect_hits arbitrary-radius 8 "rounded-sm|md|lg|full|none, rounded-tl-md and the comment block add nothing"
  expect_hits arbitrary-text-size 4 "only the bare text-[9px] counts (+2 Text.tsx, +1 Label.tsx)"
  put frontend/src/p/AfterComment.tsx $'/* opener\n*/\n<button>'
  expect_hits raw-button 3 "scanning resumes after a block comment closes"

  echo "self-test: exemptions"
  put frontend/src/p/Inline.tsx '<button>x</button> {/* ui-gate: allow hardware key */}'
  expect_hits raw-button 3 "a line containing ui-gate: allow is exempt"
  put frontend/src/components/ui/button.tsx $'<button className="rounded-xl shadow-[0_0_1px_red]" />\nimport { X } from \'lucide-react\';'
  put frontend/src/p/__tests__/Thing.tsx '<button />'
  put frontend/src/p/fixtures/data.ts '<button />'
  put frontend/src/p/Thing.test.tsx '<button />'
  put frontend/src/p/Thing.spec.ts '<button />'
  put frontend/src/app/globals.css '.u-x { @apply rounded-xl; }'
  expect_hits raw-button 3 "components/ui/, __tests__/, fixtures/, *.test.*, *.spec.* are exempt"
  expect_hits arbitrary-radius 8 "app/globals.css is exempt"
  printf 'frontend/src/p/Shadow.tsx\tarbitrary-shadow\tlamp glow\nfrontend/src/p/\traw-input\twhole directory\n' > "$sandbox/allow.txt"
  expect_hits arbitrary-shadow 0 "the allowlist exempts a file for its rule"
  expect_hits raw-input 0 "a directory entry exempts everything under it"
  printf 'frontend/src/p/Shadow.tsx\traw-button\twrong rule\n' > "$sandbox/allow.txt"
  expect_hits arbitrary-shadow 1 "an entry for another rule exempts nothing"
  printf 'frontend/src/p/Gone.tsx\traw-button\tstale\n' > "$sandbox/allow.txt"
  expect_rc 1 "an allowlist path that does not exist fails" --list raw-button
  printf 'frontend/src/p/Shadow.tsx\tno-such-rule\ttypo\n' > "$sandbox/allow.txt"
  expect_rc 1 "an allowlist entry naming an unknown rule fails" --list raw-button
  printf 'frontend/src/p/Shadow.tsx\tarbitrary-shadow\n' > "$sandbox/allow.txt"
  expect_rc 1 "an allowlist entry without a reason fails" --list raw-button
  : > "$sandbox/allow.txt"

  echo "self-test: budgets"
  expect_rc 1 "counts over an empty budget fail (a missing rule has budget 0)"
  run --list raw-button >/dev/null
  printf 'raw-button=3\nraw-input=3\narbitrary-text-size=4\narbitrary-radius=8\nlabel-override=1\narbitrary-shadow=1\nlucide-import=2\nmotion-travel=2\nhandrolled-hover=3\n' > "$sandbox/budget.txt"
  expect_rc 0 "counts equal to their budgets pass"
  put frontend/src/p/New.tsx '<button>new</button>'
  expect_rc 1 "one new hit over budget fails"
  local out; out=$(run || true)
  if echo "$out" | grep -q 'frontend/src/p/New.tsx:1'; then ok "the failure lists the offending file:line"; else fail "the failure lists the offending file:line"; echo "$out" | sed 's/^/       | /'; fi
  printf 'raw-button=junk\n' >> "$sandbox/budget.txt"
  sed -e '/^raw-button=3$/d' "$sandbox/budget.txt" > "$sandbox/b2" && mv "$sandbox/b2" "$sandbox/budget.txt"
  expect_rc 1 "a malformed budget entry fails"

  echo "self-test: --update lowers, never raises"
  printf 'raw-button=3\nraw-input=10\narbitrary-text-size=4\narbitrary-radius=8\nlabel-override=1\narbitrary-shadow=1\nlucide-import=2\nmotion-travel=2\nhandrolled-hover=3\n' > "$sandbox/budget.txt"
  run --update >/dev/null || true
  budget_is raw-input 3 "--update lowered raw-input 10 -> 3"
  budget_is raw-button 3 "--update left raw-button at 3 although the count is 4"
  expect_rc 1 "and the gate still fails until it is paid down"
  sed -e '/^motion-travel=/d' "$sandbox/budget.txt" > "$sandbox/b2" && mv "$sandbox/b2" "$sandbox/budget.txt"
  run --update >/dev/null || true
  budget_is motion-travel 0 "--update writes a missing rule as 0, not as its count"
  rm "$sandbox/frontend/src/p/New.tsx"; ( cd "$sandbox" && git add -A )
  printf 'raw-button=3\nraw-input=3\narbitrary-text-size=4\narbitrary-radius=8\nlabel-override=1\narbitrary-shadow=1\nlucide-import=2\nmotion-travel=2\nhandrolled-hover=3\n' > "$sandbox/budget.txt"
  expect_rc 0 "removing the new hit restores green"

  echo
  if [ "$failures" -gt 0 ]; then
    echo "ui-consistency gate self-test: $failures failure(s)"
    exit 1
  fi
  echo "ui-consistency gate self-test: ok"
  exit 0
}

if [ "$MODE" = selftest ]; then
  self_test
fi

# ------------------------------------------------------------------ hygiene --
problems=0
while IFS= read -r entry; do
  [ -n "$entry" ] || continue
  path=$(printf '%s' "$entry" | cut -f1)
  rule=$(printf '%s' "$entry" | cut -f2 -s)
  reason=$(printf '%s' "$entry" | cut -f3- -s)
  if [ -z "$rule" ] || [ -z "$reason" ]; then
    echo "FAIL: $ALLOW_FILE: \"$entry\" must be path<TAB>rule<TAB>reason"; problems=$((problems + 1)); continue
  fi
  if ! is_rule "$rule"; then
    echo "FAIL: $ALLOW_FILE: unknown rule \"$rule\" for $path"; problems=$((problems + 1))
  fi
  if [ ! -e "$path" ]; then
    echo "FAIL: $ALLOW_FILE lists $path, which no longer exists — remove the entry"; problems=$((problems + 1))
  fi
done < <(allow_entries)
for r in $RULE_NAMES; do
  if [ "$(budget_for "$r")" = bad ]; then
    echo "FAIL: $BUDGET_FILE: the entry for $r must be $r=<integer>"; problems=$((problems + 1))
  fi
done
if [ "$problems" -gt 0 ]; then
  echo "ui-consistency gate: $problems configuration problem(s)"
  exit 1
fi
if [ -n "$LIST_RULE" ] && ! is_rule "$LIST_RULE"; then
  echo "unknown rule \"$LIST_RULE\"; rules are:" >&2
  printf '  %s\n' $RULE_NAMES >&2
  exit 2
fi

# --------------------------------------------------------------------- scan --
HITS=$(mktemp)
trap 'rm -f "$HITS"' EXIT
scan > "$HITS"

count_for() { grep -c "^$1	" "$HITS" || true; }
hits_for() { { grep "^$1	" "$HITS" || true; } | cut -f2- | sed -e 's/	[[:space:]]*/	/' -e 's/	/  /'; }

if [ "$MODE" = list ]; then
  hits_for "$LIST_RULE"
  echo "$LIST_RULE: $(count_for "$LIST_RULE") hit(s)" >&2
  exit 0
fi

if [ "$MODE" = update ]; then
  changed=0
  new=$(mktemp)
  {
    echo "# ADR-0019 UI-consistency budget — maximum allowed hits per rule."
    echo "# Lowered by scripts/check-ui-consistency.sh --update; never raised by it."
    for r in $RULE_NAMES; do
      b=$(budget_for "$r"); c=$(count_for "$r")
      if [ "$c" -lt "$b" ]; then
        echo "budget: lowered $r $b -> $c" >&2; b=$c; changed=$((changed + 1))
      elif [ "$c" -gt "$b" ]; then
        echo "budget: NOT raised — $r has $c hit(s), budget $b; pay it down instead" >&2
      fi
      echo "$r=$b"
    done
  } > "$new"
  mv "$new" "$BUDGET_FILE"
  echo "ui-consistency gate: budget updated ($changed change(s))" >&2
  exit 0
fi

# -------------------------------------------------------------------- check --
violations=0
slack=0
printf '%-22s %6s %6s\n' rule hits budget
for r in $RULE_NAMES; do
  b=$(budget_for "$r"); c=$(count_for "$r")
  status=ok
  if [ "$c" -gt "$b" ]; then status="FAIL (+$((c - b)))"; violations=$((violations + 1))
  elif [ "$c" -lt "$b" ]; then status="ok (-$((b - c)), run --update)"; slack=1
  fi
  printf '%-22s %6s %6s  %s\n' "$r" "$c" "$b" "$status"
done

# Allowlist entries that exempted nothing are worth cleaning up, but not worth a red build.
i=0
while IFS= read -r entry; do
  i=$((i + 1))
  if ! grep -qx "ALLOWUSED	$i" "$HITS"; then
    echo "note: $ALLOW_FILE entry \"$(printf '%s' "$entry" | cut -f1-2 | tr '\t' ' ')\" exempts nothing — remove it"
  fi
done < <(allow_entries)

if [ "$violations" -gt 0 ]; then
  echo
  for r in $RULE_NAMES; do
    b=$(budget_for "$r"); c=$(count_for "$r")
    [ "$c" -gt "$b" ] || continue
    echo "FAIL: $r — $c hit(s), budget $b. Hits (the new one is among them):"
    hits_for "$r" | sed 's/^/  /'
  done
  echo
  echo "ui-consistency gate: $violations rule(s) over budget. Use the primitives and tokens in"
  echo "docs/design-system.md; a deliberate one-off takes \`// ui-gate: allow <reason>\`."
  exit 1
fi

if [ "$slack" = 1 ]; then
  echo "note: some rules are under budget — run scripts/check-ui-consistency.sh --update to lock it in"
fi
echo "ui-consistency gate: ok"
