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
#      signature, then updates/current.json, then updates/latest.json LAST (the
#      only object installed apps poll), then verify the live feed and push
#      public/release.json to the site repo. Needs CLOUDFLARE_API_TOKEN,
#      CLOUDFLARE_ACCOUNT_ID, NIXON_SITE_DIR (see SETUP.md) and
#      TAURI_SIGNING_PRIVATE_KEY (specs/0058).
#   7. Also publish a GitHub Release on the tag (notes from the changelog, DMG +
#      updater artefacts attached; needs the `gh` CLI, authenticated) for installs
#      still on the GitHub updater endpoint. Skip with --no-github.
#      --no-release skips both 6 and 7.
#
# Usage:
#   ./release.sh <major|minor|patch|X.Y.Z> [--skip-build] [--no-release] [--no-github] [--yes] [--dry-run] [--allow-degraded]
#
#   --skip-build      bump + changelog + merge/push, but don't build the DMG
#   --no-release      do everything except publish (R2 and GitHub)
#   --no-github       publish to R2 but skip the GitHub release
#   --yes             don't prompt before the merge/tag/push (remote) step
#   --dry-run         print what would happen; make no changes
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
BUMP=""

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --no-release) NO_RELEASE=1 ;;
    --no-github) ALSO_GITHUB=0 ;;
    --yes|-y) ASSUME_YES=1 ;;
    --allow-degraded) ALLOW_DEGRADED=1 ;;
    major|minor|patch) BUMP="$arg" ;;
    [0-9]*.[0-9]*.[0-9]*) BUMP="$arg" ;;
    -h|--help)
      sed -n '3,40p' "$0"; exit 0 ;;
    *) die "Unknown argument: $arg (expected major|minor|patch|X.Y.Z and optional flags)" ;;
  esac
done

[ -n "$BUMP" ] || die "Specify the bump: major | minor | patch | X.Y.Z  (see --help)"

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
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" != "HEAD" ] || die "Detached HEAD — checkout a branch to release from."

if [ -n "$(git status --porcelain)" ]; then
  die "Working tree is not clean. Commit or stash changes before releasing."
fi

c_blue "🔄 Fetching origin…"
run "git fetch --quiet origin"

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
  fi
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
c_blue "📦 Releasing ${CUR} → ${NEW}  (branch: ${BRANCH}, tag: ${TAG}, date: ${TODAY})"

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
# Non-blocking nudge to refresh docs/screenshots/real before cutting a release. This must
# run BEFORE the new tag is created below: comparing against a tag this same run just cut
# always looks "stale" (it's seconds old), so compare against the tag this release
# supersedes instead. Every substitution falls back with `|| true`/`:-0` so this can never
# trip `set -euo pipefail` on a fresh repo with no prior tag or no screenshots yet.
PREV_TAG="$(git describe --tags --abbrev=0 HEAD 2>/dev/null || true)"
if [ -n "$PREV_TAG" ]; then
  NEWEST_PNG_RAW="$(find "$REPO_ROOT/docs/screenshots/real" -name '*.png' -exec stat -f %m {} \; 2>/dev/null | sort -rn | head -1 || true)"
  NEWEST_PNG_TS="${NEWEST_PNG_RAW:-0}"
  PREV_TAG_TS="$(git log -1 --format=%ct "$PREV_TAG" 2>/dev/null || true)"
  PREV_TAG_TS="${PREV_TAG_TS:-0}"
  if [ -z "$NEWEST_PNG_RAW" ] || [ "$NEWEST_PNG_TS" -le "$PREV_TAG_TS" ]; then
    c_yellow "ℹ️  docs/screenshots/real is missing or older than ${PREV_TAG} — run pnpm shots:real to refresh the README images before releasing."
  fi
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
    python3 - "$UPDATER_PUBKEY" "$UPD_SIG" <<'PY' || keyid_status=$?
import base64, sys
def keyid(b64):
    text = base64.b64decode(b64).decode()
    line = [l for l in text.splitlines() if l and not l.startswith("untrusted comment")][0]
    return base64.b64decode(line)[2:10]
try:
    pub = keyid(sys.argv[1])
    sig = keyid(open(sys.argv[2]).read().strip())
except Exception as e:
    print(f"could not parse pubkey/.sig: {e}", file=sys.stderr)
    sys.exit(2)
sys.exit(0 if pub == sig else 1)
PY
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
  # release notes = this version's changelog section (anchored heading match,
  # so no regex escaping of the [brackets]), MINUS its "### Internal" section.
  #
  # This text is read by users twice: as the release body and as the `notes` the
  # in-app updater renders in its update dialog (see the manifests built just below). CI
  # gates, build tooling and refactors do not belong in either, so the changelog parks
  # them under "### Internal" and they are dropped here. Dropping at publish time rather
  # than asking the author to remember keeps the changelog complete while keeping the
  # release notes about the app.
  NOTES_FILE="$(mktemp)"
  awk -v ver="$NEW" '
    index($0, "## [" ver "]") == 1 { f=1; next }
    f && /^## \[/ { f=0 }
    f && /^### Internal[[:space:]]*$/ { skip=1; next }
    f && skip && /^### / { skip=0 }
    f && !skip { print }
  ' "$CHANGELOG" > "$NOTES_FILE"
  [ -s "$NOTES_FILE" ] || printf 'Nixon v%s\n' "$NEW" > "$NOTES_FILE"

  MANIFEST_DIR="$(mktemp -d)"
  MANIFEST_GH="${MANIFEST_DIR}/latest.json"        # the GitHub-hosted feed (legacy installs)
  MANIFEST_R2="${MANIFEST_DIR}/latest.r2.json"     # the nixonapp.com feed
  if [ "$SKIP_BUILD" -eq 0 ]; then
    # specs/0058: the manifest the installed app polls. Versioned asset URL so an
    # older manifest can never point at a newer tarball. Emitted twice (specs/0080):
    # once pointing at GitHub, once at nixonapp.com/R2.
    build_manifest() { # out url
      python3 - "$NEW" "$NOTES_FILE" "$UPD_SIG" "$TAG" "$1" "$DRY_RUN" "$2" <<'PY'
import json, sys, datetime, os
ver, notes_file, sig_file, tag, out, dry, url = sys.argv[1:8]
sig = open(sig_file).read().strip() if os.path.exists(sig_file) else ("<signature>" if dry == "1" else "")
if not sig: sys.exit("missing updater signature")
manifest = {
  "version": ver,
  "notes": open(notes_file).read().strip(),
  "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
  "platforms": {"darwin-aarch64": {
    "signature": sig,
    "url": url}},
}
json.dump(manifest, open(out, "w"), indent=2)
PY
    }
    build_manifest "$MANIFEST_GH" "https://github.com/sxates/nixon/releases/download/${TAG}/Nixon.app.tar.gz"
    build_manifest "$MANIFEST_R2" "${SITE_BASE}/releases/${NEW}/Nixon.app.tar.gz"

    # ---- R2 (specs/0080) -----------------------------------------------------
    # Interrupted-release safety: immutable, versioned artifacts go up FIRST; then
    # updates/current.json (download page pointer); updates/latest.json goes LAST,
    # because that is the only object installed apps poll. Any failure before it
    # leaves nothing advertised.
    # wrangler runs with cwd = the site repo (pnpm --dir), so every --file must be absolute.
    abs_path() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$REPO_ROOT" "$1" ;; esac; }
    DMG_NAME="Nixon_${NEW}_aarch64.dmg"
    CURRENT_JSON="${MANIFEST_DIR}/current.json"
    printf '{"version":"%s","dmg":"releases/%s/%s"}\n' "$NEW" "$NEW" "$DMG_NAME" > "$CURRENT_JSON"
    # key|file|content-type — in upload order; latest.json LAST: it is what installed apps see.
    R2_PLAN=(
      "releases/${NEW}/${DMG_NAME}|$(abs_path "$DMG")|application/x-apple-diskimage"
      "releases/${NEW}/Nixon.app.tar.gz|$(abs_path "$UPD_TGZ")|application/gzip"
      "releases/${NEW}/Nixon.app.tar.gz.sig|$(abs_path "$UPD_SIG")|text/plain"
      "updates/current.json|${CURRENT_JSON}|application/json"
      "updates/latest.json|${MANIFEST_R2}|application/json"
    )
    GH_RETRY="gh release create '$TAG' '$(abs_path "$DMG")' '$(abs_path "$UPD_TGZ")' '$(abs_path "$UPD_SIG")' '$MANIFEST_GH' --title 'Nixon v${NEW}' --notes-file '$NOTES_FILE'"
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
   State: tag ${TAG} and main are ALREADY pushed; the GitHub release has NOT been created; nothing new is advertised on the R2 feed unless updates/latest.json was already uploaded.
   Recovery — run these in order (latest.json last), then the GitHub step:
$(r2_manual_steps)
     ${GH_RETRY}"
    }

    c_blue "☁️  Uploading to R2 (${R2_BUCKET})…"
    for e in "${R2_PLAN[@]}"; do
      IFS='|' read -r k f t <<<"$e"
      r2_put "$k" "$f" "$t"
    done

    if [ "$DRY_RUN" -eq 0 ]; then
      c_blue "🔎 Verifying the live feed at ${SITE_BASE}…"
      CURL_RETRY=( --retry 3 --retry-delay 2 --retry-connrefused --max-time 30 )
      verify_die() {
        die "$1
   Skipped because of this: the GitHub release and the site release.json push. Artifacts and the feed are already uploaded.
   To finish: re-check ${SITE_BASE}/updates/latest.json, then run:
     ${GH_RETRY}
   and update ${NIXON_SITE_DIR}/public/release.json by hand."
      }
      live="$(curl -fsS "${CURL_RETRY[@]}" "${SITE_BASE}/updates/latest.json?current_version=0.0.0&target=verify" | python3 -c 'import json,sys;print(json.load(sys.stdin)["version"])')" \
        || verify_die "Uploaded, but ${SITE_BASE}/updates/latest.json did not respond. Investigate before announcing."
      [ "$live" = "$NEW" ] || verify_die "Feed serves $live, expected $NEW."
      want="$(stat -f%z "$DMG")"
      got="$(curl -fsSI "${CURL_RETRY[@]}" "${SITE_BASE}/releases/${NEW}/${DMG_NAME}" | awk 'tolower($1)=="content-length:"{print $2+0}')" \
        || verify_die "Could not HEAD ${SITE_BASE}/releases/${NEW}/${DMG_NAME}."
      [ "$want" = "$got" ] || verify_die "DMG size mismatch on ${SITE_BASE}: local $want, served ${got:-none}."
      want_t="$(stat -f%z "$UPD_TGZ")"
      got_t="$(curl -fsSI "${CURL_RETRY[@]}" "${SITE_BASE}/releases/${NEW}/Nixon.app.tar.gz" | awk 'tolower($1)=="content-length:"{print $2+0}')" \
        || verify_die "Could not HEAD ${SITE_BASE}/releases/${NEW}/Nixon.app.tar.gz."
      [ "$want_t" = "$got_t" ] || verify_die "Updater tarball size mismatch on ${SITE_BASE}: local $want_t, served ${got_t:-none}."
      c_green "   ✅ Feed serves ${NEW}; DMG and updater tarball sizes match."
    fi

    # ---- site release.json (page shows the current version; warn, never fail) ----
    if [ "$DRY_RUN" -eq 1 ]; then
      echo "  [dry-run] write ${NIXON_SITE_DIR}/public/release.json and push"
    else
      (
        set -e
        size="$(stat -f%z "$DMG")"
        printf '{"version":"%s","size":%s,"date":"%s"}\n' "$NEW" "$size" "$(date -u +%F)" > "${NIXON_SITE_DIR}/public/release.json"
        git -C "$NIXON_SITE_DIR" add public/release.json
        if ! git -C "$NIXON_SITE_DIR" diff --cached --quiet -- public/release.json; then
          git -C "$NIXON_SITE_DIR" commit -qm "release: v${NEW}"
        fi
        git -C "$NIXON_SITE_DIR" push -q origin main
      ) || c_yellow "⚠️  Could not push release.json to the site repo; the page will show the old version until you do."
    fi
  else
    c_yellow "   (--skip-build: no build artefacts — nothing uploaded to R2; installed apps will NOT see this release)"
  fi

  # ---- GitHub release (optional; --no-github skips) ---------------------------
  if [ "$ALSO_GITHUB" -eq 1 ]; then
    c_blue "🚀 Publishing GitHub release ${TAG}…"
    rel_args=( "$TAG" --title "Nixon v${NEW}" --notes-file "$NOTES_FILE" )
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
        || { rm -f "$NOTES_FILE"; die "gh release create failed. Tag ${TAG} is already pushed — retry with: gh release create '$TAG' '$DMG' '$UPD_TGZ' '$UPD_SIG' '$MANIFEST_GH' --title 'Nixon v${NEW}' --generate-notes"; }
    fi
  else
    c_yellow "⏭️  --no-github: skipping the GitHub release. Installs still on the GitHub endpoint will not see this release."
  fi
  if [ "$DRY_RUN" -eq 1 ] && [ -f "$MANIFEST_R2" ]; then
    echo "  [dry-run] updates/latest.json (R2):"
    sed 's/^/    /' "$MANIFEST_R2"
    echo
  fi
  rm -f "$NOTES_FILE"
fi

c_green "✅ Released v${NEW}."
if [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "   DMG: ${REPO_ROOT}/target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"
fi
echo "   Tag: ${TAG} (pushed to origin)"
if [ "$NO_RELEASE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "   Installed apps will pick this update up on their own (in-app updater, specs/0058)."
  echo "   Download page / install on another Mac: ${SITE_BASE}/download/"
  if [ "$ALSO_GITHUB" -eq 1 ]; then
    echo "   GitHub: gh release download '$TAG' -R sxates/nixon && open Nixon_${NEW}_aarch64.dmg"
  fi
fi
