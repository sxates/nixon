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
#   6. Publish a GitHub Release on the tag, with notes from the changelog and
#      the DMG attached (needs the `gh` CLI, authenticated). Skipped with
#      --no-release; download builds elsewhere via `gh release download`.
#      Publishes DMG + updater tarball + latest.json; needs
#      TAURI_SIGNING_PRIVATE_KEY (specs/0058).
#
# Usage:
#   ./release.sh <major|minor|patch|X.Y.Z> [--skip-build] [--no-release] [--yes] [--dry-run] [--allow-degraded]
#
#   --skip-build      bump + changelog + merge/push, but don't build the DMG
#   --no-release      do everything except publish the GitHub release
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
ASSUME_YES=0
ALLOW_DEGRADED=0
BUMP=""

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --no-release) NO_RELEASE=1 ;;
    --yes|-y) ASSUME_YES=1 ;;
    --allow-degraded) ALLOW_DEGRADED=1 ;;
    major|minor|patch) BUMP="$arg" ;;
    [0-9]*.[0-9]*.[0-9]*) BUMP="$arg" ;;
    -h|--help)
      sed -n '3,33p' "$0"; exit 0 ;;
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
if [ "$NO_RELEASE" -eq 0 ]; then
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

# ---- publish the GitHub release -------------------------------------------
if [ "$NO_RELEASE" -eq 1 ]; then
  c_yellow "⏭️  --no-release: not publishing a GitHub release. Installed apps won't see this update until it's published. Publish later with:"
  echo "     gh release create '$TAG' '$DMG' '$UPD_TGZ' '$UPD_SIG' --title 'Nixon v${NEW}' --generate-notes  # then build+attach latest.json (see release.sh's publish step)"
else
  c_blue "🚀 Publishing GitHub release ${TAG}…"
  # release notes = this version's changelog section (anchored heading match,
  # so no regex escaping of the [brackets])
  NOTES_FILE="$(mktemp)"
  awk -v ver="$NEW" '
    index($0, "## [" ver "]") == 1 { f=1; next }
    f && /^## \[/ { f=0 }
    f { print }
  ' "$CHANGELOG" > "$NOTES_FILE"
  [ -s "$NOTES_FILE" ] || printf 'Nixon v%s\n' "$NEW" > "$NOTES_FILE"

  rel_args=( "$TAG" --title "Nixon v${NEW}" --notes-file "$NOTES_FILE" )
  MANIFEST="$(mktemp -d)/latest.json"
  if [ "$SKIP_BUILD" -eq 0 ]; then
    # specs/0058: the manifest the installed app polls. Versioned asset URL so an
    # older manifest can never point at a newer tarball.
    python3 - "$NEW" "$NOTES_FILE" "$UPD_SIG" "$TAG" "$MANIFEST" "$DRY_RUN" <<'PY'
import json, sys, datetime, os
ver, notes_file, sig_file, tag, out, dry = sys.argv[1:7]
sig = open(sig_file).read().strip() if os.path.exists(sig_file) else ("<signature>" if dry == "1" else "")
if not sig: sys.exit("missing updater signature")
manifest = {
  "version": ver,
  "notes": open(notes_file).read().strip(),
  "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
  "platforms": {"darwin-aarch64": {
    "signature": sig,
    "url": f"https://github.com/sxates/nixon/releases/download/{tag}/Nixon.app.tar.gz"}},
}
json.dump(manifest, open(out, "w"), indent=2)
PY
    rel_args+=( "$DMG" "$UPD_TGZ" "$UPD_SIG" "$MANIFEST" )
  else
    # Only --skip-build reaches this now: on a real publish a missing DMG, updater
    # tarball or signature already died above, so an asset-less release can no longer
    # happen by accident.
    c_yellow "   (--skip-build: no build artefacts to attach — this release will have no assets and installed apps will NOT see it)"
  fi

  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  [dry-run] gh release create ${rel_args[*]}"
    if [ -f "$MANIFEST" ]; then
      echo "  [dry-run] latest.json:"
      sed 's/^/    /' "$MANIFEST"
      echo
    fi
  else
    gh release create "${rel_args[@]}" \
      || { rm -f "$NOTES_FILE"; die "gh release create failed. Tag ${TAG} is already pushed — retry with: gh release create '$TAG' '$DMG' '$UPD_TGZ' '$UPD_SIG' '$MANIFEST' --title 'Nixon v${NEW}' --generate-notes"; }
  fi
  rm -f "$NOTES_FILE"
fi

c_green "✅ Released v${NEW}."
if [ "$DRY_RUN" -eq 0 ]; then
  # Non-blocking: just a nudge to refresh the README's real screenshots before/after
  # cutting a release. `git tag -a` (above) creates a loose ref, so the common case is a
  # plain mtime comparison; fall back to comparing commit timestamps numerically if the
  # ref has since been packed (e.g. this tag was cut by an earlier, separate run).
  TAG_REF="$REPO_ROOT/.git/refs/tags/${TAG}"
  if [ -f "$TAG_REF" ]; then
    STALE_SHOTS="$(find "$REPO_ROOT/docs/screenshots/real" -name '*.png' -newer "$TAG_REF" 2>/dev/null)"
  else
    TAG_TS="$(git -C "$REPO_ROOT" log -1 --format=%ct "$TAG" 2>/dev/null || echo 0)"
    NEWEST_PNG_TS="$(find "$REPO_ROOT/docs/screenshots/real" -name '*.png' -exec stat -f %m {} \; 2>/dev/null | sort -rn | head -1)"
    NEWEST_PNG_TS="${NEWEST_PNG_TS:-0}"
    [ "$NEWEST_PNG_TS" -gt "$TAG_TS" ] && STALE_SHOTS="present" || STALE_SHOTS=""
  fi
  if [ -z "$STALE_SHOTS" ]; then
    c_yellow "ℹ️  docs/screenshots/real is older than this tag — run pnpm shots:real to refresh the README images"
  fi
fi
if [ "$SKIP_BUILD" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "   DMG: ${REPO_ROOT}/target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"
fi
echo "   Tag: ${TAG} (pushed to origin)"
if [ "$NO_RELEASE" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "   Installed apps will pick this update up on their own (in-app updater, specs/0058)."
  echo "   Install on another Mac (avoids Gatekeeper quarantine):"
  echo "     gh release download '$TAG' -R sxates/nixon && open Nixon_${NEW}_aarch64.dmg"
fi
