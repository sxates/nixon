#!/usr/bin/env bash
#
# release.sh — cut a Nixon production release.
#
# Does, in order:
#   1. Bump the version (major | minor | patch, or an explicit X.Y.Z) across
#      package.json, tauri.conf.json, src-tauri/Cargo.toml, and Cargo.lock.
#   2. Roll CHANGELOG.md: move the [Unreleased] notes under a new
#      "[X.Y.Z] - <date>" heading and reset [Unreleased]. Aborts if [Unreleased]
#      is empty so you never ship a blank entry (Keep a Changelog discipline).
#   3. Commit the bump on the current branch.
#   4. Build the production DMG (frontend/build-gpu.sh, Metal).
#   5. Merge the current branch into main, tag it vX.Y.Z, and push both
#      (main + tag) to origin.
#   6. Publish to Cloudflare R2 (nixonapp.com, specs/0080): DMG + updater tarball +
#      signature, then updates/current.json, then updates/early.json (only if it
#      would move forward, specs/0086), then updates/latest.json LAST (the feed
#      every install polls), then verify the live feed and update the site
#      (scripts/publish-site.sh: release.json + screenshots, push, deploy). Needs CLOUDFLARE_API_TOKEN,
#      CLOUDFLARE_ACCOUNT_ID, NIXON_SITE_DIR (see SETUP.md) and
#      TAURI_SIGNING_PRIVATE_KEY (specs/0058).
#   7. Also publish a GitHub Release on the tag (notes from the changelog, DMG +
#      updater artefacts attached; needs the `gh` CLI, authenticated) for installs
#      still on the GitHub updater endpoint. Skip with --no-github.
#      --no-release skips both 6 and 7.
#
# Release notes (every feed write): each changelog section newer than the version
# that feed advertises now, up to this one, newest first, with "### Internal"
# stripped. Usually that is just this version's section (specs/0086 DECIDED-2).
#
# Usage:
#   ./release.sh <major|minor|patch|X.Y.Z> [--early] [--skip-build] [--no-release] [--no-github] [--yes] [--dry-run] [--allow-degraded]
#   ./release.sh promote X.Y.Z [--no-github] [--yes] [--dry-run]
#
#   --early           specs/0086: the full release above, but advertised ONLY on
#                     updates/early.json (installs with Settings > Updates >
#                     Early access updates on). updates/latest.json,
#                     updates/current.json and the site's release.json are not
#                     touched; the GitHub release is a pre-release. Cannot be
#                     combined with --skip-build or --no-release.
#   promote X.Y.Z     specs/0086: give an early build to everyone. Nothing is
#                     rebuilt or committed: it checks that vX.Y.Z's files are on
#                     R2, that X.Y.Z is newer than updates/latest.json, that the
#                     signature matches the app's key (and the GitHub
#                     pre-release's), then writes updates/current.json, then
#                     updates/latest.json LAST, pushes release.json, and turns the
#                     GitHub pre-release into the latest full release.
#   --skip-build      bump + changelog + merge/push, but don't build the DMG
#   --no-release      do everything except publish (R2 and GitHub)
#   --no-github       publish to R2 but skip the GitHub release
#   --yes             don't prompt before the remote steps
#   --dry-run         print what would happen; make no changes. The read-only
#                     checks (live feeds, R2 HEADs, gh release view) still run.
#   --allow-degraded  override the safety preflight: build even without the
#                     Google client id (inert Calendar) and/or publish without
#                     Developer ID + notarization. For local/test builds only —
#                     NEVER for a real web-distributed release. Without it, a run
#                     that would ship a broken build aborts (this is the guard
#                     that a hand-run build-gpu.sh lacks — always release here).
#
set -euo pipefail

# ---- helpers ---------------------------------------------------------------
c_blue() { printf '\033[0;34m%s\033[0m\n' "$*"; }
c_green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
c_yellow() { printf '\033[1;33m%s\033[0m\n' "$*"; }
die() { printf '\033[0;31m❌ %s\033[0m\n' "$*" >&2; exit 1; }

DRY_RUN=0
SKIP_BUILD=0
NO_RELEASE=0
ALSO_GITHUB=1
ASSUME_YES=0
ALLOW_DEGRADED=0
EARLY=0
MODE="release"
BUMP=""

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --no-release) NO_RELEASE=1 ;;
    --no-github) ALSO_GITHUB=0 ;;
    --yes|-y) ASSUME_YES=1 ;;
    --allow-degraded) ALLOW_DEGRADED=1 ;;
    --early) EARLY=1 ;;
    promote) MODE="promote" ;;
    major|minor|patch) BUMP="$arg" ;;
    [0-9]*.[0-9]*.[0-9]*) BUMP="$arg" ;;
    -h|--help)
      # the header comment, from line 3 to the first non-comment line
      awk 'NR<3{next} /^#/{print; next} {exit}' "$0"; exit 0 ;;
    *) die "Unknown argument: $arg (expected major|minor|patch|X.Y.Z, or promote X.Y.Z, and optional flags)" ;;
  esac
done

PROMOTE_VER=""
if [ "$MODE" = "promote" ]; then
  [[ "$BUMP" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "promote needs an exact version: ./release.sh promote X.Y.Z"
  PROMOTE_VER="$BUMP"; BUMP=""
  [ "$EARLY" -eq 0 ] || die "--early and promote are opposites; pick one."
  [ "$SKIP_BUILD" -eq 0 ] && [ "$NO_RELEASE" -eq 0 ] || die "promote publishes an existing build; --skip-build and --no-release do not apply."
  [ "$ALLOW_DEGRADED" -eq 0 ] || die "promote builds nothing, so --allow-degraded does not apply."
else
  [ -n "$BUMP" ] || die "Specify the bump: major | minor | patch | X.Y.Z, or promote X.Y.Z  (see --help)"
  if [ "$EARLY" -eq 1 ]; then
    # the point of --early is to publish a build that installs can receive
    [ "$NO_RELEASE" -eq 0 ] || die "--early publishes a build to the early feed; it cannot be combined with --no-release."
    [ "$SKIP_BUILD" -eq 0 ] || die "--early publishes a build to the early feed; it cannot be combined with --skip-build."
  fi
fi

run() {
  if [ "$DRY_RUN" -eq 1 ]; then echo "  [dry-run] $*"; else eval "$@"; fi
}

# ---- locate repo root ------------------------------------------------------
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "Not in a git repository."
cd "$REPO_ROOT"

PKG_JSON="frontend/package.json"
TAURI_CONF="frontend/src-tauri/tauri.conf.json"
CARGO_TOML="frontend/src-tauri/Cargo.toml"
CARGO_LOCK="Cargo.lock"
CHANGELOG="CHANGELOG.md"
for f in "$PKG_JSON" "$TAURI_CONF" "$CARGO_TOML" "$CARGO_LOCK" "$CHANGELOG"; do
  [ -f "$f" ] || die "Missing expected file: $f"
done

# ---- pre-flight ------------------------------------------------------------
# promote changes nothing in this repo (no bump, no commit, no tag), so it needs neither a
# branch nor a clean tree; it fetches tags itself, for real even in --dry-run (read-only).
if [ "$MODE" = "release" ]; then
  BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  [ "$BRANCH" != "HEAD" ] || die "Detached HEAD — checkout a branch to release from."

  if [ -n "$(git status --porcelain)" ]; then
    die "Working tree is not clean. Commit or stash changes before releasing."
  fi

  c_blue "🔄 Fetching origin…"
  run "git fetch --quiet origin"
fi

# gh is needed for the GitHub release step — check now so we fail fast (before build)
if [ "$NO_RELEASE" -eq 0 ] && [ "$ALSO_GITHUB" -eq 1 ]; then
  command -v gh >/dev/null 2>&1 || die "gh CLI not found (needed to publish the release). Install it ('brew install gh') and 'gh auth login', or pass --no-release."
  if [ "$DRY_RUN" -eq 0 ]; then
    gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run 'gh auth login', or pass --no-release."
  fi
fi

# ---- code-signing / notarization (optional) -------------------------------
# If .env.signing exists, load Developer ID + notarization creds so `tauri build`
# (via build-gpu.sh) signs + notarizes + staples the DMG → installable from a
# normal web download. Absent → ad-hoc signing (install via `gh release
# download`). See docs/decisions/ADR-0008 and SETUP.md "Signing & notarization".
SIGNING_ENV="${SIGNING_ENV:-.env.signing}"
if [ -f "$SIGNING_ENV" ]; then
  c_blue "🔏 Loading signing credentials from ${SIGNING_ENV}…"
  set -a; # shellcheck disable=SC1090
  . "$SIGNING_ENV"; set +a
fi

# ---- R2 / nixonapp.com publishing (specs/0080) ----------------------------
# wrangler runs from the site repo's pinned install (pnpm --dir "$NIXON_SITE_DIR" exec).
# Credentials come from .env.signing (or the environment); export them so wrangler sees them.
R2_BUCKET="nixon-releases"
SITE_BASE="https://nixonapp.com"
if [ "$NO_RELEASE" -eq 0 ]; then
  : "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN in ${SIGNING_ENV} (see SETUP.md)}"
  : "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID in ${SIGNING_ENV} (see SETUP.md)}"
  : "${NIXON_SITE_DIR:?set NIXON_SITE_DIR in ${SIGNING_ENV} (see SETUP.md)}"
  export CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  [ -d "$NIXON_SITE_DIR" ] || die "NIXON_SITE_DIR ($NIXON_SITE_DIR) does not exist"
  command -v pnpm >/dev/null 2>&1 || die "pnpm not found (needed to run wrangler from the site repo)"
  if [ "$SKIP_BUILD" -eq 0 ]; then
    # fail before the build/tag, not after the tag is pushed
    [ -d "$NIXON_SITE_DIR/public" ] || die "$NIXON_SITE_DIR/public does not exist (is NIXON_SITE_DIR the site repo?)"
    pnpm --dir "$NIXON_SITE_DIR" exec wrangler --version >/dev/null 2>&1 || die "wrangler not runnable via 'pnpm --dir $NIXON_SITE_DIR exec wrangler' (run pnpm install in the site repo)."
    # The site step (scripts/publish-site.sh) needs this; catch it before the build, not after the push.
    if [ "$EARLY" -eq 0 ]; then
      site_branch="$(git -C "$NIXON_SITE_DIR" rev-parse --abbrev-ref HEAD)"
      [ "$site_branch" = "${NIXON_SITE_BRANCH:-main}" ] || die "site repo is on '$site_branch', not '${NIXON_SITE_BRANCH:-main}' (NIXON_SITE_BRANCH in ${SIGNING_ENV}); switch it first."
      [ -z "$(git -C "$NIXON_SITE_DIR" status --porcelain)" ] || die "site repo ($NIXON_SITE_DIR) has uncommitted changes; commit or stash them first."
    fi
    # Token preflight: before the build/tag, prove the token can reach the bucket. A missing
    # updates/latest.json (brand-new empty bucket) is fine; only auth/permission errors fail.
    if [ "$DRY_RUN" -eq 1 ]; then
      echo "  [dry-run] would verify the Cloudflare token"
    else
      cf_err="$(pnpm --dir "$NIXON_SITE_DIR" exec wrangler r2 object get "$R2_BUCKET/updates/latest.json" --remote --pipe 2>&1 >/dev/null)" || {
        if printf '%s' "$cf_err" | grep -Eiq '401|403|authentication|unauthori[sz]ed|permission|forbidden|invalid.*token|10000|10042'; then
          die "Cloudflare token cannot read $R2_BUCKET (check CLOUDFLARE_API_TOKEN / account id; the token needs R2 Object Read & Write)"
        fi
      }
    fi
  fi
fi

# ---- shared publish helpers (specs/0086) ----------------------------------
# The pure manifest / notes / version / key-id logic lives in scripts/release_feed.py
# (CI runs its --self-test). What is here is the I/O around it, shared by a normal
# release, --early and promote.
feed() { python3 "$REPO_ROOT/scripts/release_feed.py" "$@"; }
# wrangler runs with cwd = the site repo (pnpm --dir), so every --file must be absolute.
abs_path() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$REPO_ROOT" "$1" ;; esac; }
CURL_RETRY=( --retry 3 --retry-delay 2 --retry-connrefused --max-time 30 )

# read_live <latest|early>: GET updates/<feed>.json through the Worker (no-cache) and
# classify it with release_feed.py feed-read. Sets LIVE_STATE (ok | missing | unknown |
# refuse), LIVE_VER and LIVE_MSG. The Worker serves latest.json whenever early.json is
# absent, so a 404/503 on early.json is a fault, and so is one on latest.json unless
# NIXON_RELEASE_EMPTY_FEED=1 says this is the first release into an empty bucket (normal
# releases only). A fault is "refuse" in a real run (the caller stops or collects it) and
# "unknown" in a dry run, which continues with <unknown>. A feed is never moved backwards
# because of a bad read.
read_live() {
  local body code out opts=()
  body="$(mktemp)"
  code="$(curl -sS --retry 2 --retry-delay 1 --max-time 20 -o "$body" -w '%{http_code}' \
    "${SITE_BASE}/updates/$1.json?current_version=0.0.0&target=verify" 2>/dev/null)" || true
  [ "$DRY_RUN" -eq 1 ] && opts+=( --dry-run )
  if [ "$1" = "latest" ] && [ "$MODE" = "release" ] && [ "${NIXON_RELEASE_EMPTY_FEED:-0}" = "1" ]; then
    opts+=( --allow-missing )
  fi
  out="$(feed feed-read --feed "$1" --http "${code:-000}" --body "$body" ${opts[@]+"${opts[@]}"})" \
    || { rm -f "$body"; die "Could not classify the read of updates/$1.json (is python3 working?)."; }
  rm -f "$body"
  { IFS= read -r LIVE_STATE; IFS= read -r LIVE_MSG; } <<<"$out" || true
  LIVE_VER=""
  case "$LIVE_STATE" in
    ok) LIVE_VER="$LIVE_MSG"; c_green "   Live updates/$1.json: ${LIVE_VER}" ;;
    missing) c_yellow "   Live updates/$1.json: not published (NIXON_RELEASE_EMPTY_FEED=1) — treated as older than any version." ;;
    unknown) LIVE_VER="<unknown>"; c_yellow "   ⚠️  ${LIVE_MSG} [dry-run] continuing with <unknown>; a real run would stop here." ;;
    *) LIVE_STATE="refuse" ;;
  esac
}

# R2 uploads, in R2_PLAN order ("key|file|content-type"). The caller sets R2_PLAN and the
# R2_FAIL_* text so a failed upload names the state and the remaining steps for its mode.
r2_manual_steps() {
  local e k f t
  for e in "${R2_PLAN[@]}"; do
    IFS='|' read -r k f t <<<"$e"
    echo "     pnpm --dir '$NIXON_SITE_DIR' exec wrangler r2 object put '$R2_BUCKET/$k' --file '$f' --content-type '$t' --remote"
  done
}
r2_put() { # key file content-type
  local f; f="$(abs_path "$2")"
  if [ "$DRY_RUN" -eq 1 ]; then echo "  [dry-run] r2 put $R2_BUCKET/$1 <- $f ($3)"; return 0; fi
  pnpm --dir "$NIXON_SITE_DIR" exec wrangler r2 object put "$R2_BUCKET/$1" \
    --file "$f" --content-type "$3" --remote >/dev/null \
    || die "R2 upload failed for $1.
   State: ${R2_FAIL_STATE}
   Recovery — run these in order (${R2_FAIL_LAST} last), then ${R2_FAIL_NEXT}:
$(r2_manual_steps)
${R2_FAIL_TAIL}"
}
r2_upload_plan() {
  local e k f t
  c_blue "☁️  Uploading to R2 (${R2_BUCKET})…"
  for e in "${R2_PLAN[@]}"; do
    IFS='|' read -r k f t <<<"$e"
    r2_put "$k" "$f" "$t"
  done
}

# verify_live <updates/NAME.json> <version> <dmg name> <dmg bytes> <tarball bytes>: the
# feed serves <version> and both files are served at the expected size. Failures go to
# the caller's verify_die, which says what was skipped and how to finish.
verify_live() {
  local live got got_t
  c_blue "🔎 Verifying the live feed at ${SITE_BASE}…"
  live="$(curl -fsS "${CURL_RETRY[@]}" "${SITE_BASE}/$1?current_version=0.0.0&target=verify" | feed version)" \
    || verify_die "Uploaded, but ${SITE_BASE}/$1 did not respond. Investigate before announcing."
  [ "$live" = "$2" ] || verify_die "Feed serves $live, expected $2."
  got="$(curl -fsSI "${CURL_RETRY[@]}" "${SITE_BASE}/releases/$2/$3" | awk 'tolower($1)=="content-length:"{print $2+0}')" \
    || verify_die "Could not HEAD ${SITE_BASE}/releases/$2/$3."
  [ "$4" = "$got" ] || verify_die "DMG size mismatch on ${SITE_BASE}: expected $4, served ${got:-none}."
  got_t="$(curl -fsSI "${CURL_RETRY[@]}" "${SITE_BASE}/releases/$2/Nixon.app.tar.gz" | awk 'tolower($1)=="content-length:"{print $2+0}')" \
    || verify_die "Could not HEAD ${SITE_BASE}/releases/$2/Nixon.app.tar.gz."
  [ "$5" = "$got_t" ] || verify_die "Updater tarball size mismatch on ${SITE_BASE}: expected $5, served ${got_t:-none}."
  c_green "   ✅ $1 serves $2; DMG and updater tarball sizes match."
}

# ---- nixonapp.com: release.json + screenshots + deploy (warn, never fail) ----
# scripts/publish-site.sh does the work so it can also be re-run by hand (a second
# promote refuses once latest.json moved). It needs the site repo on NIXON_SITE_BRANCH.
publish_release_json() { # version dmg-bytes
  "$REPO_ROOT/scripts/publish-site.sh" "$1" --size "$2"
}
RELEASE_JSON_WARN="⚠️  nixonapp.com was not updated (see the error above). Fix it, then run: scripts/publish-site.sh <version>"

# ---- screenshot_reminder <since-tag> (specs/0060) ---------------------------
# Non-blocking nudge to refresh docs/screenshots/real when they are missing or older than
# <since-tag>. Every substitution falls back with `|| true`/`:-0` so this can never trip
# `set -euo pipefail` on a fresh repo with no prior tag or no screenshots yet.
screenshot_reminder() {
  local since="$1" newest_raw newest_ts since_ts
  [ -n "$since" ] || return 0
  git rev-parse -q --verify "refs/tags/$since" >/dev/null 2>&1 || return 0
  newest_raw="$(find "$REPO_ROOT/docs/screenshots/real" -name '*.png' -exec stat -f %m {} \; 2>/dev/null | sort -rn | head -1 || true)"
  newest_ts="${newest_raw:-0}"
  since_ts="$(git log -1 --format=%ct "$since" 2>/dev/null || true)"
  since_ts="${since_ts:-0}"
  if [ -z "$newest_raw" ] || [ "$newest_ts" -le "$since_ts" ]; then
    c_yellow "ℹ️  docs/screenshots/real is missing or older than ${since} — run pnpm shots:real to refresh the README images before this version reaches everyone."
  fi
}

# ---- promote X.Y.Z (specs/0086) -------------------------------------------
# Gives an existing early build to everyone. Builds, signs and commits nothing; every
# preflight check is read-only and runs in --dry-run too. Writes, in order:
# updates/current.json, updates/early.json (only if it is somehow older), then
# updates/latest.json LAST; verify; site release.json; GitHub latest.json asset; and
# finally the GitHub pre-release -> latest flip (what legacy GitHub-endpoint installs see).
if [ "$MODE" = "promote" ]; then
  # shellcheck source=scripts/release-promote.sh
  . "$REPO_ROOT/scripts/release-promote.sh"
  exit 0
fi

# ---- Google Calendar OAuth client (optional, specs/0032) -------------------
# Compile-time option_env! values; without them the Google provider is inert
# ("not configured" in Settings). Git-ignored; same file dev-nixon.sh sources.
GOOGLE_ENV="${GOOGLE_ENV:-frontend/src-tauri/.env.google}"
if [ -f "$GOOGLE_ENV" ]; then
  c_blue "📅 Loading Google OAuth client from ${GOOGLE_ENV}…"
  set -a; # shellcheck disable=SC1090
  . "$GOOGLE_ENV"; set +a
fi
if [ -n "${NIXON_GOOGLE_CLIENT_ID:-}" ]; then
  c_green "   Google Calendar: client id present (${#NIXON_GOOGLE_CLIENT_ID} chars)"
elif [ "$SKIP_BUILD" -eq 1 ]; then
  c_yellow "   Google Calendar: no client id — but --skip-build, so no binary is produced."
elif [ "$ALLOW_DEGRADED" -eq 1 ]; then
  c_yellow "   Google Calendar: no client id — feature will be INERT in this build (--allow-degraded)."
elif [ "$DRY_RUN" -eq 1 ]; then
  c_yellow "   Google Calendar: no client id — [dry-run] a real run would ABORT here."
else
  die "Google client id missing: ${GOOGLE_ENV} absent or NIXON_GOOGLE_CLIENT_ID empty.
   This is exactly what shipped an inert-Calendar 1.7.0 (Settings said 'not configured',
   no attendees). option_env! bakes it at compile time, so the binary would be broken.
   Fix: restore ${GOOGLE_ENV} (see SETUP.md 'Google Calendar'), or pass --allow-degraded
   to intentionally build without Calendar."
fi

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  SIGN_MODE="developerid"
  if { [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; } \
     || { [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; }; then
    NOTARIZE=1
  else
    NOTARIZE=0
  fi
  c_green "   Signing identity: ${APPLE_SIGNING_IDENTITY}"
  if [ "$NOTARIZE" -eq 1 ]; then
    c_green "   Notarization: enabled"
  else
    c_yellow "   Notarization: no creds (need APPLE_API_KEY+APPLE_API_ISSUER, or APPLE_ID+APPLE_PASSWORD+APPLE_TEAM_ID) — will sign but NOT notarize."
  fi
  # fail fast if the identity isn't actually in the keychain (before the long build)
  if [ "$DRY_RUN" -eq 0 ] \
     && ! security find-identity -v -p codesigning 2>/dev/null | grep -qF "$APPLE_SIGNING_IDENTITY"; then
    die "APPLE_SIGNING_IDENTITY '${APPLE_SIGNING_IDENTITY}' not in keychain (check: security find-identity -v -p codesigning). Import the Developer ID Application cert + key first."
  fi
else
  SIGN_MODE="adhoc"
  c_yellow "🔏 Ad-hoc signing (no APPLE_SIGNING_IDENTITY) — not notarized. For web-distributable builds see SETUP.md 'Signing & notarization'."
fi

# A *published* release must be Developer ID signed AND notarized — a web
# download of an ad-hoc or un-notarized DMG is quarantined and blocked by
# Gatekeeper (this shipped once already). Abort before the long build unless
# explicitly overridden. --no-release/--skip-build/--dry-run don't publish, so
# they only warn above; --allow-degraded is the deliberate override.
if [ "$NO_RELEASE" -eq 0 ] && [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ] && [ "$ALLOW_DEGRADED" -eq 0 ]; then
  [ "$SIGN_MODE" = "developerid" ] || die "Publishing a release needs Developer ID signing (APPLE_SIGNING_IDENTITY via ${SIGNING_ENV}). A web download of an ad-hoc build is blocked by Gatekeeper. Restore the creds, or pass --no-release for a local build, or --allow-degraded to override."
  [ "${NOTARIZE:-0}" -eq 1 ] || die "Publishing a release needs notarization creds (APPLE_API_KEY+APPLE_API_ISSUER, or APPLE_ID+APPLE_PASSWORD+APPLE_TEAM_ID via ${SIGNING_ENV}). Restore the creds, or --no-release, or --allow-degraded."
fi

# ---- updater signing key (specs/0058) -------------------------------------
# tauri build signs Nixon.app.tar.gz with the minisign key in TAURI_SIGNING_PRIVATE_KEY;
# installed apps refuse any payload not signed by the pubkey in tauri.conf.json. An
# unsigned update is not "degraded", it is undeliverable — so --allow-degraded does
# NOT waive this. Only --no-release / --skip-build / --dry-run get past it.
if [ "$NO_RELEASE" -eq 0 ] && [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || die "TAURI_SIGNING_PRIVATE_KEY is not set (add it to ${SIGNING_ENV}; see SETUP.md 'In-app updates'). Without it the release cannot be delivered to installed apps."
fi
UPDATER_PUBKEY_PLACEHOLDER=0
UPDATER_PUBKEY="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["plugins"]["updater"]["pubkey"])' "$TAURI_CONF" 2>/dev/null || true)"
if [ -z "$UPDATER_PUBKEY" ]; then
  if [ "$NO_RELEASE" -eq 0 ] && [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
    die "plugins.updater.pubkey missing from ${TAURI_CONF}."
  else
    c_yellow "   plugins.updater.pubkey missing from ${TAURI_CONF} — a real publish would abort here; continuing with a placeholder pubkey for this dry-run/local build."
    UPDATER_PUBKEY="<pubkey-not-yet-configured>"
    UPDATER_PUBKEY_PLACEHOLDER=1
  fi
fi

# ---- compute the new version ----------------------------------------------
CUR="$(node -p "require('./$PKG_JSON').version" 2>/dev/null)" \
  || die "Couldn't read current version from $PKG_JSON"
[[ "$CUR" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Current version '$CUR' isn't X.Y.Z"

IFS='.' read -r MAJ MIN PAT <<<"$CUR"
case "$BUMP" in
  major) NEW="$((MAJ+1)).0.0" ;;
  minor) NEW="${MAJ}.$((MIN+1)).0" ;;
  patch) NEW="${MAJ}.${MIN}.$((PAT+1))" ;;
  *)     NEW="$BUMP" ;;
esac
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Computed version '$NEW' isn't X.Y.Z"
[ "$NEW" != "$CUR" ] || die "New version equals current ($CUR); nothing to bump."

TAG="v${NEW}"
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
  die "Tag ${TAG} already exists."
fi
DMG="target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"

TODAY="$(date +%F)"
if [ "$EARLY" -eq 1 ]; then
  c_blue "📦 Releasing ${CUR} → ${NEW} EARLY  (branch: ${BRANCH}, tag: ${TAG}, date: ${TODAY}; advertised on updates/early.json only)"
else
  c_blue "📦 Releasing ${CUR} → ${NEW}  (branch: ${BRANCH}, tag: ${TAG}, date: ${TODAY})"
fi

# ---- live feeds (specs/0086) ------------------------------------------------
# A feed is only ever rewritten forward, and updates/latest.json is always written last.
# Read both live feeds now, before anything changes: they decide the notes (each feed's
# notes are every changelog section after the version it advertises) and whether
# updates/early.json is written at all.
LIVE_LATEST=""
LIVE_EARLY=""
EARLY_WRITE=0
if [ "$NO_RELEASE" -eq 0 ]; then
  c_blue "📡 Reading the live feeds at ${SITE_BASE}…"
  read_live latest
  [ "$LIVE_STATE" != "refuse" ] || die "${LIVE_MSG} Nothing has been changed yet."
  LIVE_LATEST="$LIVE_VER"
  # --early: this is also the preflight that the Worker serves /updates/early.json
  # (specs/0086 W0), before the build. A normal release needs it for the forward guard.
  read_live early
  [ "$LIVE_STATE" != "refuse" ] || die "${LIVE_MSG} Nothing has been changed yet."
  LIVE_EARLY="$LIVE_VER"
  if [ "$EARLY" -eq 1 ]; then
    feed newer "$NEW" "$LIVE_EARLY" \
      || die "${NEW} is not newer than the live early feed (${LIVE_EARLY}); a feed is only ever moved forward."
    EARLY_WRITE=1
  else
    feed newer "$NEW" "$LIVE_LATEST" \
      || die "${NEW} is not newer than the live everyone feed (${LIVE_LATEST}); a feed is only ever moved forward."
    # specs/0086: early.json always advertises the newer of the early and everyone builds.
    # A normal release is newest in practice (versions bump from package.json), but the
    # guard means a re-run or a hand-edited bucket can never move early.json backwards.
    if feed newer "$NEW" "$LIVE_EARLY"; then
      EARLY_WRITE=1
    else
      c_yellow "   updates/early.json already advertises ${LIVE_EARLY} (≥ ${NEW}); it will not be rewritten."
    fi
  fi
fi

# ---- verify [Unreleased] has content --------------------------------------
UNRELEASED_BODY="$(awk '/^## \[Unreleased\]/{f=1;next} f&&/^## \[/{f=0} f' "$CHANGELOG")"
TRIMMED="$(printf '%s' "$UNRELEASED_BODY" | sed 's/_(nothing yet)_//Ig' | tr -d '[:space:]')"
[ -n "$TRIMMED" ] || die "CHANGELOG.md [Unreleased] is empty — add release notes there first."

# ---- bump version files (format-preserving) -------------------------------
c_blue "✍️  Bumping version in manifests…"
bump_json() {  # first "version": "<cur>" only
  CUR="$CUR" NEW="$NEW" perl -i -pe 'BEGIN{$d=0} if(!$d && /"version":\s*"\Q$ENV{CUR}\E"/){ s/"version":\s*"\Q$ENV{CUR}\E"/"version": "$ENV{NEW}"/; $d=1 }' "$1"
}
if [ "$DRY_RUN" -eq 0 ]; then
  bump_json "$PKG_JSON"
  bump_json "$TAURI_CONF"
  CUR="$CUR" NEW="$NEW" perl -i -pe 's/^version = "\Q$ENV{CUR}\E"/version = "$ENV{NEW}"/' "$CARGO_TOML"
  # Cargo.lock: the version line that follows  name = "nixon"
  CUR="$CUR" NEW="$NEW" perl -i -pe 'if($p && /^version = "\Q$ENV{CUR}\E"/){ s/^version = "\Q$ENV{CUR}\E"/version = "$ENV{NEW}"/; $p=0 } $p=1 if /^name = "nixon"$/' "$CARGO_LOCK"
  # sanity: each manifest now carries the new version
  grep -q "\"version\": \"$NEW\"" "$PKG_JSON"   || die "package.json bump failed"
  grep -q "\"version\": \"$NEW\"" "$TAURI_CONF" || die "tauri.conf.json bump failed"
  grep -q "^version = \"$NEW\""  "$CARGO_TOML"  || die "Cargo.toml bump failed"
else
  echo "  [dry-run] would set version=$NEW in $PKG_JSON, $TAURI_CONF, $CARGO_TOML, $CARGO_LOCK"
fi

# ---- roll the changelog ----------------------------------------------------
c_blue "📝 Rolling CHANGELOG.md ([Unreleased] → [$NEW] - $TODAY)…"
if [ "$DRY_RUN" -eq 0 ]; then
  tmp="$(mktemp)"
  awk -v ver="$NEW" -v date="$TODAY" '
    state==2 { print; next }
    state==1 {
      if ($0 ~ /^## \[/) {
        print "";
        print "_(nothing yet)_";
        print "";
        print "## [" ver "] - " date;
        printf "%s", body;
        print $0;
        state=2; next
      }
      body = body $0 "\n"; next
    }
    { print; if ($0 ~ /^## \[Unreleased\]/) { state=1; body="" } }
  ' "$CHANGELOG" > "$tmp"
  mv "$tmp" "$CHANGELOG"
  grep -q "^## \[$NEW\] - $TODAY" "$CHANGELOG" || die "Changelog roll failed"
else
  echo "  [dry-run] would move [Unreleased] notes under ## [$NEW] - $TODAY"
fi

# ---- commit the bump -------------------------------------------------------
c_blue "💾 Committing release bump…"
run "git add '$PKG_JSON' '$TAURI_CONF' '$CARGO_TOML' '$CARGO_LOCK' '$CHANGELOG'"
run "git commit -m 'release: v${NEW}'"

# ---- screenshot freshness reminder (specs/0060) ---------------------------
# Only when a version reaches everyone: a normal release here, and promote (which calls
# screenshot_reminder itself). Early releases never nudge — README images track the
# everyone track. It runs BEFORE the new tag is created below: comparing against a tag
# this same run just cut always looks "stale" (it's seconds old), so compare against the
# tag this release supersedes instead.
if [ "$EARLY" -eq 0 ]; then
  screenshot_reminder "$(git describe --tags --abbrev=0 HEAD 2>/dev/null || true)"
fi

# ---- build the production DMG ----------------------------------------------
if [ "$SKIP_BUILD" -eq 0 ]; then
  c_blue "🏗️  Building production DMG (this takes a while)…"
  # shellcheck disable=SC1090
  [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
  if [ "$DRY_RUN" -eq 0 ]; then
    ( cd frontend && ./build-gpu.sh ) || die "Build failed. The release commit is local only (not pushed) — fix and re-run, or 'git reset --hard HEAD~1' to undo the bump."
    # specs/0058: a *published* release missing the DMG is a release nobody can install,
    # and the warning was easy to scroll past. Only --no-release (local build) warns.
    if [ ! -f "$DMG" ]; then
      if [ "$NO_RELEASE" -eq 1 ]; then
        c_yellow "⚠️  Build finished but expected DMG not found at $DMG"
      else
        die "Build finished but the DMG is missing at $DMG. Refusing to publish a release without it — check the bundle step and re-run."
      fi
    fi
  else
    echo "  [dry-run] would run frontend/build-gpu.sh → target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"
  fi
else
  c_yellow "⏭️  --skip-build: not building the DMG."
fi

# ---- notarize + staple the DMG itself -------------------------------------
# Tauri notarizes/staples the .app but ships it inside an un-notarized DMG.
# Staple the container too so a *browser* download passes Gatekeeper offline.
if [ "${SIGN_MODE:-adhoc}" = "developerid" ] && [ "${NOTARIZE:-0}" -eq 1 ] \
   && [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ] && [ -f "$DMG" ] \
   && ! xcrun stapler validate "$DMG" >/dev/null 2>&1; then
  c_blue "📤 Notarizing + stapling the DMG…"
  if [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
    nt_auth=( --key "${APPLE_API_KEY_PATH:?APPLE_API_KEY_PATH not set}" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" )
  else
    nt_auth=( --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" )
  fi
  # Don't gate the staple on parsing notarytool's output (its format burned us in
  # v1.3.0 — Accepted, but the grep missed and the DMG shipped unstapled). Stapling
  # itself is the authoritative check: it only succeeds if Apple accepted the DMG.
  NT_OUT="$(xcrun notarytool submit "$DMG" "${nt_auth[@]}" --wait 2>&1)" \
    || c_yellow "   ⚠️  notarytool submit exited non-zero for the DMG."
  printf '%s\n' "$NT_OUT" | tail -5
  if xcrun stapler staple "$DMG"; then
    c_green "   ✅ DMG notarized + stapled."
  else
    c_yellow "   ⚠️  DMG staple failed (notarization not accepted yet?). The .app inside is still notarized + stapled. Retry later with: xcrun stapler staple '$DMG' && gh release upload <tag> '$DMG' --clobber"
  fi
fi

# ---- verify signature + notarization --------------------------------------
if [ "${SIGN_MODE:-adhoc}" = "developerid" ] && [ "$SKIP_BUILD" -eq 0 ] \
   && [ "$DRY_RUN" -eq 0 ] && [ -f "$DMG" ]; then
  c_blue "🔍 Verifying Developer ID signature + notarization…"
  APP="target/release/bundle/macos/Nixon.app"
  if [ -d "$APP" ]; then
    codesign -dv --verbose=4 "$APP" 2>&1 | grep -E 'Authority=|flags=' | head -4 || true
    if spctl -a -vvv -t install "$APP" 2>&1 | grep -q "source=Notarized Developer ID"; then
      c_green "   ✅ Gatekeeper: accepted, Notarized Developer ID."
    else
      c_yellow "   ⚠️  Gatekeeper did not report 'Notarized Developer ID' for $APP (signed but maybe not notarized)."
    fi
  fi
  if xcrun stapler validate "$DMG" >/dev/null 2>&1; then
    c_green "   ✅ Notarization ticket stapled to the DMG."
  else
    c_yellow "   ⚠️  DMG has no stapled ticket — it will require online Gatekeeper checks (or isn't notarized)."
  fi
fi

# ---- updater artefacts (specs/0058) ---------------------------------------
UPD_TGZ="target/release/bundle/macos/Nixon.app.tar.gz"
UPD_SIG="${UPD_TGZ}.sig"
if [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  [ -s "$UPD_TGZ" ] || die "Updater tarball missing: $UPD_TGZ (is bundle.createUpdaterArtifacts true?)"
  [ -s "$UPD_SIG" ] || die "Updater signature missing: $UPD_SIG (was TAURI_SIGNING_PRIVATE_KEY set for the build?)"
  if [ "$UPDATER_PUBKEY_PLACEHOLDER" -eq 1 ]; then
    c_yellow "   plugins.updater.pubkey isn't configured yet — skipping the signature key-id check (this build isn't a real publish: --no-release/--skip-build/--dry-run)."
  else
    # Both are base64-wrapped minisign blobs; the key id is bytes 2..10 of the second
    # line's payload. A mismatch means installed apps would reject this release.
    # Exit 2 = couldn't parse either blob (not a mismatch); exit 1 = ids differ; 0 = match.
    keyid_status=0
    feed keyid-match --pubkey "$UPDATER_PUBKEY" --sig "$UPD_SIG" || keyid_status=$?
    case "$keyid_status" in
      0) c_green "   ✅ Updater artefacts present; signature key id matches the app's pubkey." ;;
      2) die "Could not parse plugins.updater.pubkey or ${UPD_SIG} as minisign blobs. Check TAURI_CONF and TAURI_SIGNING_PRIVATE_KEY." ;;
      127) die "python3 not found — the updater signature key-id check can't run. Install python3 (it is also used to build latest.json) and re-run." ;;
      *) die "Updater signature was made with a different key than plugins.updater.pubkey. Check TAURI_SIGNING_PRIVATE_KEY." ;;
    esac
  fi
fi

# ---- confirm before remote ops --------------------------------------------
if [ "$ASSUME_YES" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  printf '\nAbout to merge %s → main, tag %s, and push both to origin. Continue? [y/N] ' "$BRANCH" "$TAG"
  read -r ans || true
  case "$ans" in y|Y|yes|YES) ;; *) die "Aborted before merge/push. Release commit (v${NEW}) is local on ${BRANCH}." ;; esac
fi

# ---- merge to main, tag, push ---------------------------------------------
if [ "$BRANCH" != "main" ]; then
  c_blue "🔀 Merging ${BRANCH} → main…"
  run "git checkout main"
  run "git pull --ff-only origin main"
  run "git merge --no-ff '$BRANCH' -m 'Merge ${BRANCH} for release v${NEW}'"
else
  c_yellow "Already on main; skipping merge."
fi

c_blue "🏷️  Tagging ${TAG}…"
run "git tag -a '$TAG' -m 'Nixon v${NEW}'"

c_blue "⬆️  Pushing main + tag to origin…"
run "git push origin main"
run "git push origin '$TAG'"
# keep the (now-merged) working branch in sync too
if [ "$BRANCH" != "main" ]; then
  run "git push origin '$BRANCH'"
  run "git checkout '$BRANCH'"
fi

# ---- publish: R2 (nixonapp.com) + GitHub ------------------------------------
if [ "$NO_RELEASE" -eq 1 ]; then
  c_yellow "⏭️  --no-release: not publishing. Installed apps won't see this update until it's published (R2 uploads + updates/latest.json, and optionally a GitHub release; see release.sh's publish step)."
  echo "     GitHub-only fallback: gh release create '$TAG' '$DMG' '$UPD_TGZ' '$UPD_SIG' --title 'Nixon v${NEW}' --generate-notes"
else
  # Release notes come from the changelog (scripts/release_feed.py notes): every section
  # newer than the version the feed advertises now, up to this one, newest first, MINUS
  # each section's "### Internal". Usually that is just this version's section; it grows
  # only when an early build was never promoted (specs/0086 DECIDED-2).
  #
  # This text is read by users twice: as the release body and as the `notes` the
  # in-app updater renders in its update dialog (see the manifests built just below). CI
  # gates, build tooling and refactors do not belong in either, so the changelog parks
  # them under "### Internal" and they are dropped here. Dropping at publish time rather
  # than asking the author to remember keeps the changelog complete while keeping the
  # release notes about the app.
  NOTES_LATEST="$(mktemp)"   # for updates/latest.json (everyone)
  NOTES_EARLY="$(mktemp)"    # for updates/early.json
  feed notes --changelog "$CHANGELOG" --upto "$NEW" --after "$LIVE_LATEST" --out "$NOTES_LATEST" \
    || die "Could not build the release notes from $CHANGELOG."
  feed notes --changelog "$CHANGELOG" --upto "$NEW" --after "$LIVE_EARLY" --out "$NOTES_EARLY" \
    || die "Could not build the release notes from $CHANGELOG."
  # the GitHub release body: an early release is announced to early installs
  if [ "$EARLY" -eq 1 ]; then NOTES_FILE="$NOTES_EARLY"; else NOTES_FILE="$NOTES_LATEST"; fi

  MANIFEST_DIR="$(mktemp -d)"
  MANIFEST_GH="${MANIFEST_DIR}/latest.json"        # the GitHub-hosted feed (legacy installs)
  MANIFEST_R2="${MANIFEST_DIR}/latest.r2.json"     # the nixonapp.com feed
  MANIFEST_EARLY="${MANIFEST_DIR}/early.r2.json"   # the nixonapp.com early feed (specs/0086)
  GH_PRE=""
  [ "$EARLY" -eq 1 ] && GH_PRE=" --prerelease --latest=false"
  if [ "$SKIP_BUILD" -eq 0 ]; then
    # specs/0058: the manifest the installed app polls. Versioned asset URL so an
    # older manifest can never point at a newer tarball. Emitted per feed (specs/0080,
    # specs/0086): GitHub, nixonapp.com latest, nixonapp.com early.
    build_manifest() { # out url notes
      local dry=()
      [ "$DRY_RUN" -eq 1 ] && dry=( --dry-run )
      feed manifest --version "$NEW" --notes "$3" --sig "$UPD_SIG" --url "$2" --out "$1" ${dry[@]+"${dry[@]}"} \
        || die "Could not build $1 (missing updater signature?)"
    }
    build_manifest "$MANIFEST_GH" "https://github.com/sxates/nixon/releases/download/${TAG}/Nixon.app.tar.gz" "$NOTES_FILE"
    [ "$EARLY" -eq 1 ] || build_manifest "$MANIFEST_R2" "${SITE_BASE}/releases/${NEW}/Nixon.app.tar.gz" "$NOTES_LATEST"
    [ "$EARLY_WRITE" -eq 1 ] && build_manifest "$MANIFEST_EARLY" "${SITE_BASE}/releases/${NEW}/Nixon.app.tar.gz" "$NOTES_EARLY"

    # ---- R2 (specs/0080, specs/0086) -------------------------------------------
    # Interrupted-release safety: immutable, versioned artifacts go up FIRST; then
    # updates/current.json (download page pointer); then updates/early.json (only if it
    # moves forward); updates/latest.json goes LAST, because it is the feed every install
    # polls. Any failure before it leaves nothing new advertised to everyone.
    # --early stops at updates/early.json: latest.json and current.json are not touched.
    DMG_NAME="Nixon_${NEW}_aarch64.dmg"
    CURRENT_JSON="${MANIFEST_DIR}/current.json"
    printf '{"version":"%s","dmg":"releases/%s/%s"}\n' "$NEW" "$NEW" "$DMG_NAME" > "$CURRENT_JSON"
    # key|file|content-type — in upload order; the advertised feed LAST.
    R2_PLAN=(
      "releases/${NEW}/${DMG_NAME}|$(abs_path "$DMG")|application/x-apple-diskimage"
      "releases/${NEW}/Nixon.app.tar.gz|$(abs_path "$UPD_TGZ")|application/gzip"
      "releases/${NEW}/Nixon.app.tar.gz.sig|$(abs_path "$UPD_SIG")|text/plain"
    )
    [ "$EARLY" -eq 1 ] || R2_PLAN+=( "updates/current.json|${CURRENT_JSON}|application/json" )
    [ "$EARLY_WRITE" -eq 1 ] && R2_PLAN+=( "updates/early.json|${MANIFEST_EARLY}|application/json" )
    [ "$EARLY" -eq 1 ] || R2_PLAN+=( "updates/latest.json|${MANIFEST_R2}|application/json" )
    GH_RETRY="gh release create '$TAG' '$(abs_path "$DMG")' '$(abs_path "$UPD_TGZ")' '$(abs_path "$UPD_SIG")' '$MANIFEST_GH' --title 'Nixon v${NEW}' --notes-file '$NOTES_FILE'${GH_PRE}"
    if [ "$EARLY" -eq 1 ]; then
      ADVERTISED="updates/early.json"
      R2_FAIL_STATE="tag ${TAG} and main are ALREADY pushed; the GitHub pre-release has NOT been created; nothing reached the everyone track (updates/latest.json, updates/current.json and release.json are untouched), and nothing new is on the early feed unless updates/early.json was already uploaded."
      R2_FAIL_LAST="early.json"
    else
      ADVERTISED="updates/latest.json"
      R2_FAIL_STATE="tag ${TAG} and main are ALREADY pushed; the GitHub release has NOT been created; nothing new is advertised on the R2 feed unless updates/latest.json was already uploaded."
      R2_FAIL_LAST="latest.json"
    fi
    R2_FAIL_NEXT="the GitHub step"
    R2_FAIL_TAIL="     ${GH_RETRY}"

    r2_upload_plan
    if [ "$DRY_RUN" -eq 1 ] && [ "$EARLY" -eq 1 ]; then
      echo "  [dry-run] not touched: updates/latest.json, updates/current.json, public/release.json"
    fi

    if [ "$DRY_RUN" -eq 0 ]; then
      if [ "$EARLY" -eq 1 ]; then
        verify_die() {
          die "$1
   Skipped because of this: the GitHub pre-release. Artifacts and updates/early.json are already uploaded. Nothing reached the everyone track.
   To finish: re-check ${SITE_BASE}/updates/early.json, then run:
     ${GH_RETRY}"
        }
      else
        verify_die() {
          die "$1
   Skipped because of this: the GitHub release and the site release.json push. Artifacts and the feed are already uploaded.
   To finish: re-check ${SITE_BASE}/updates/latest.json, then run:
     ${GH_RETRY}
   and run scripts/publish-site.sh ${NEW}."
        }
      fi
      verify_live "$ADVERTISED" "$NEW" "$DMG_NAME" "$(stat -f%z "$DMG")" "$(stat -f%z "$UPD_TGZ")"
    fi

    # ---- site release.json (the download page shows the everyone version) ----------
    if [ "$EARLY" -eq 1 ]; then
      c_yellow "   --early: public/release.json not touched (the download page keeps the everyone version)."
    elif [ "$DRY_RUN" -eq 1 ]; then
      "$REPO_ROOT/scripts/publish-site.sh" "$NEW" --size 1 --dry-run || c_yellow "$RELEASE_JSON_WARN"
    else
      { dmg_size="$(stat -f%z "$DMG")" && publish_release_json "$NEW" "$dmg_size"; } || c_yellow "$RELEASE_JSON_WARN"
    fi
  else
    c_yellow "   (--skip-build: no build artefacts — nothing uploaded to R2; installed apps will NOT see this release)"
  fi

  # ---- GitHub release (optional; --no-github skips) ---------------------------
  if [ "$ALSO_GITHUB" -eq 1 ]; then
    if [ "$EARLY" -eq 1 ]; then
      c_blue "🚀 Publishing GitHub pre-release ${TAG}…"
    else
      c_blue "🚀 Publishing GitHub release ${TAG}…"
    fi
    rel_args=( "$TAG" --title "Nixon v${NEW}" --notes-file "$NOTES_FILE" )
    # specs/0086: GitHub's releases/latest/download/latest.json never resolves to a
    # pre-release, so legacy installs on the GitHub endpoint never see an early build.
    [ "$EARLY" -eq 1 ] && rel_args+=( --prerelease --latest=false )
    if [ "$SKIP_BUILD" -eq 0 ]; then
      rel_args+=( "$DMG" "$UPD_TGZ" "$UPD_SIG" "$MANIFEST_GH" )
    else
      # Only --skip-build reaches this now: on a real publish a missing DMG, updater
      # tarball or signature already died above, so an asset-less release can no longer
      # happen by accident.
      c_yellow "   (--skip-build: no build artefacts to attach — this release will have no assets and installed apps will NOT see it)"
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
      echo "  [dry-run] gh release create ${rel_args[*]}"
      if [ -f "$MANIFEST_GH" ]; then
        echo "  [dry-run] latest.json (GitHub):"
        sed 's/^/    /' "$MANIFEST_GH"
        echo
      fi
    else
      gh release create "${rel_args[@]}" \
        || { rm -f "$NOTES_LATEST" "$NOTES_EARLY"; die "gh release create failed. Tag ${TAG} is already pushed — retry with: gh release create '$TAG' '$DMG' '$UPD_TGZ' '$UPD_SIG' '$MANIFEST_GH' --title 'Nixon v${NEW}' --generate-notes${GH_PRE}"; }
    fi
  else
    c_yellow "⏭️  --no-github: skipping the GitHub release. Installs still on the GitHub endpoint will not see this release."
  fi
  if [ "$DRY_RUN" -eq 1 ] && [ -f "$MANIFEST_R2" ]; then
    echo "  [dry-run] updates/latest.json (R2):"
    sed 's/^/    /' "$MANIFEST_R2"
    echo
  fi
  if [ "$DRY_RUN" -eq 1 ] && [ -f "$MANIFEST_EARLY" ]; then
    echo "  [dry-run] updates/early.json (R2):"
    sed 's/^/    /' "$MANIFEST_EARLY"
    echo
  fi
  rm -f "$NOTES_LATEST" "$NOTES_EARLY"
fi

if [ "$EARLY" -eq 1 ]; then
  c_green "✅ Released v${NEW} early. Promote with ./release.sh promote ${NEW}"
else
  c_green "✅ Released v${NEW}."
fi
if [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "   DMG: ${REPO_ROOT}/target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"
fi
echo "   Tag: ${TAG} (pushed to origin)"
if [ "$NO_RELEASE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  if [ "$EARLY" -eq 1 ]; then
    echo "   Installs with Early access updates on will pick this up on their own; everyone else stays on ${LIVE_LATEST:-their version}."
    echo "   Install on another Mac by hand: the DMG on the GitHub pre-release ${TAG}."
  else
    echo "   Installed apps will pick this update up on their own (in-app updater, specs/0058)."
    echo "   Download page / install on another Mac: ${SITE_BASE}/download/"
  fi
  if [ "$ALSO_GITHUB" -eq 1 ]; then
    echo "   GitHub: gh release download '$TAG' -R sxates/nixon && open Nixon_${NEW}_aarch64.dmg"
  fi
fi
