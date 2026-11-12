#!/usr/bin/env bash
#
# publish-site.sh — bring nixonapp.com up to date with the version everyone has.
#
# Called by release.sh (normal release and promote; never --early), and safe to run by
# hand to finish or redo that step. In the site repo ($NIXON_SITE_DIR) it:
#   1. checks out nothing: the repo must already be on $NIXON_SITE_BRANCH (default main)
#      with a clean tree, then pulls --ff-only;
#   2. writes public/release.json (version, DMG size, date) — the download page reads it;
#   3. syncs the site screenshots from docs/screenshots/real (resized to 1600 px wide;
#      deck = dark, faceplate = light) and bumps their ?v= cache-busters when they change;
#   4. commits whatever changed, pushes the branch;
#   5. deploys production (nixonapp.com) and staging (staging.nixonapp.com). Pushing alone
#      deploys nothing.
# Re-running with the same inputs commits nothing and just redeploys.
#
# Usage: scripts/publish-site.sh X.Y.Z [--size BYTES] [--no-screenshots] [--no-deploy] [--dry-run]
#   --size   DMG size in bytes; default: HEAD of nixonapp.com/releases/X.Y.Z/<dmg>.
#
# Env (from .env.signing when run by hand): NIXON_SITE_DIR, NIXON_SITE_BRANCH,
# CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE_BASE="https://nixonapp.com"
die() { printf '\033[0;31m❌ %s\033[0m\n' "$*" >&2; exit 1; }
say() { printf '\033[0;34m%s\033[0m\n' "$*"; }

VER="" SIZE="" SHOTS=1 DEPLOY=1 DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --size) SIZE="${2:-}"; shift ;;
    --no-screenshots) SHOTS=0 ;;
    --no-deploy) DEPLOY=0 ;;
    --dry-run) DRY=1 ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    *) [ -z "$VER" ] || die "unexpected argument: $1"; VER="$1" ;;
  esac
  shift
done
[[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: scripts/publish-site.sh X.Y.Z [--size BYTES] [--no-screenshots] [--no-deploy] [--dry-run]"

if [ -z "${NIXON_SITE_DIR:-}" ] && [ -f "$REPO_ROOT/.env.signing" ]; then
  set -a; # shellcheck disable=SC1091
  . "$REPO_ROOT/.env.signing"; set +a
fi
: "${NIXON_SITE_DIR:?set NIXON_SITE_DIR in .env.signing (see SETUP.md)}"
BRANCH="${NIXON_SITE_BRANCH:-main}"
SITE="$NIXON_SITE_DIR"
[ -d "$SITE/public" ] || die "$SITE/public does not exist (is NIXON_SITE_DIR the site repo?)"
g() { git -C "$SITE" "$@"; }

# ---- 1. branch + clean tree -------------------------------------------------
cur="$(g rev-parse --abbrev-ref HEAD)"
[ "$cur" = "$BRANCH" ] || die "site repo is on '$cur', not '$BRANCH' (NIXON_SITE_BRANCH). Switch it, then re-run: scripts/publish-site.sh $VER"
[ -z "$(g status --porcelain)" ] || die "site repo has uncommitted changes; commit or stash them, then re-run: scripts/publish-site.sh $VER"
if [ "$DRY" -eq 0 ]; then g pull -q --ff-only origin "$BRANCH" || die "could not fast-forward $BRANCH from origin"; fi

# ---- 2. release.json --------------------------------------------------------
if [ -z "$SIZE" ]; then
  SIZE="$(curl -sfI "${SITE_BASE}/releases/${VER}/Nixon_${VER}_aarch64.dmg" | awk 'tolower($1)=="content-length:"{print $2}' | tr -d '\r' | tail -1)"
fi
[[ "$SIZE" =~ ^[0-9]+$ ]] && [ "$SIZE" -gt 0 ] || die "no DMG size for $VER (pass --size, or check ${SITE_BASE}/releases/${VER}/)"
RJ="{\"version\":\"${VER}\",\"size\":${SIZE},\"date\":\"$(date -u +%F)\"}"

if [ "$DRY" -eq 1 ]; then
  echo "  [dry-run] $SITE/public/release.json <- $RJ"
else
  # Same version + size already there: keep its date, so a re-run commits nothing.
  old="$(cat "$SITE/public/release.json" 2>/dev/null || true)"
  if [ "${old%,\"date\"*}" != "${RJ%,\"date\"*}" ]; then printf '%s\n' "$RJ" > "$SITE/public/release.json"; fi
fi

# ---- 3. screenshots ---------------------------------------------------------
# site asset stem | capture in docs/screenshots/real
SHOT_MAP=( "nixon-today|meetings-week" "nixon-summary|meeting-summary" "nixon-prep|meeting-prep" )
if [ "$SHOTS" -eq 1 ]; then
  for pair in "${SHOT_MAP[@]}"; do
    stem="${pair%%|*}" src="${pair##*|}"
    for v in "deck|" "faceplate|-light"; do
      from="$REPO_ROOT/docs/screenshots/real/${src}.${v%%|*}.png" to="$SITE/public/assets/${stem}${v##*|}.png"
      [ -f "$from" ] || { echo "  (no $from; keeping the site's ${stem}${v##*|}.png)"; continue; }
      if [ "$DRY" -eq 1 ]; then echo "  [dry-run] $from -> $to (1600 px)"; continue; fi
      sips -Z 1600 "$from" --out "$to" >/dev/null || die "sips failed on $from"
    done
  done
  if [ "$DRY" -eq 0 ]; then
    # sips output is byte-stable, so only a real recapture shows up as a change.
    stamp="$(date -u +%Y%m%d)"
    for stem in $(g diff --name-only -- public/assets | sed -n 's#^public/assets/\(nixon-[a-z]*\)\(-light\)\{0,1\}\.png$#\1#p' | sort -u); do
      grep -rlE "/assets/${stem}(-light)?\.png\?v=[0-9]+" "$SITE/public" --include='*.html' | while read -r f; do
        sed -i '' -E "s#(/assets/${stem}(-light)?\.png\?v=)[0-9]+#\1${stamp}#g" "$f"
      done
      say "   screenshots: ${stem} updated"
    done
  fi
fi

# ---- 4. commit + push -------------------------------------------------------
if [ "$DRY" -eq 1 ]; then
  echo "  [dry-run] commit 'release: v${VER}' on $BRANCH and push"
elif [ -n "$(g status --porcelain)" ]; then
  g add -A public
  g commit -qm "release: v${VER}"
  g push -q origin "$BRANCH"
  say "   site repo: committed and pushed 'release: v${VER}' on $BRANCH"
else
  say "   site repo: already up to date for v${VER}"
fi

# ---- 5. deploy --------------------------------------------------------------
if [ "$DEPLOY" -eq 1 ]; then
  : "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN in .env.signing}"
  : "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID in .env.signing}"
  export CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  for target in deploy deploy:staging; do
    if [ "$DRY" -eq 1 ]; then echo "  [dry-run] pnpm --dir $SITE $target"; continue; fi
    out="$(pnpm --dir "$SITE" "$target" 2>&1)" || { printf '%s\n' "$out" | tail -20 >&2; die "pnpm $target failed; fix it, then re-run: scripts/publish-site.sh $VER"; }
    say "   deployed ($target)"
  done
fi

printf '\033[0;32m✅ nixonapp.com shows v%s (%s MB)\033[0m\n' "$VER" "$((SIZE / 1048576))"
