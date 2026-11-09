# shellcheck shell=bash
#
# release-promote.sh — the `./release.sh promote X.Y.Z` mode (specs/0086).
#
# Sourced by release.sh after its argument parsing, .env.signing load and the Cloudflare
# credential preflight; never run directly. It relies on release.sh's variables
# (PROMOTE_VER, DRY_RUN, ASSUME_YES, ALSO_GITHUB, SITE_BASE, R2_BUCKET, TAURI_CONF,
# NIXON_SITE_DIR) and helpers (feed, read_live, live_feed_version, r2_upload_plan,
# verify_live, publish_release_json, c_*, die).
#
# It promotes the exact signed build that went out with --early: nothing is rebuilt,
# re-signed or committed, and CHANGELOG.md is read at the tag, never edited.
#
# Every preflight check is read-only and runs in --dry-run too. Refusals are collected
# and reported together. A read that fails (offline) stops a real run; a dry run warns
# and goes on with <unknown>.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "release-promote.sh is part of release.sh; run ./release.sh promote X.Y.Z" >&2
  exit 1
fi

P="$PROMOTE_VER"
PTAG="v${P}"
PDMG="Nixon_${P}_aarch64.dmg"
PWORK="$(mktemp -d)"
REFUSALS=()

refuse() { REFUSALS+=("$*"); printf '\033[0;31m   ✗ %s\033[0m\n' "$*"; }
unknown() {
  if [ "$DRY_RUN" -eq 1 ]; then
    c_yellow "   ⚠️  $* — [dry-run] continuing with <unknown>; a real run would stop here."
  else
    die "$* Nothing has been changed — check the connection and re-run."
  fi
}

# r2_head <name>: HEAD ${SITE_BASE}/releases/P/<name>. Sets HEAD_CODE and HEAD_LEN.
r2_head() {
  local hdr
  hdr="$(mktemp)"
  HEAD_CODE="$(curl -sS -I --retry 2 --retry-delay 1 --max-time 20 -o "$hdr" -w '%{http_code}' \
    "${SITE_BASE}/releases/${P}/$1" 2>/dev/null)" || HEAD_CODE="000"
  HEAD_LEN="$(awk 'tolower($1)=="content-length:"{print $2+0}' "$hdr" | tail -1)"
  rm -f "$hdr"
}

c_blue "🚚 Promoting ${PTAG} to everyone (nothing is rebuilt)…"

# ---- 1. the tag exists ------------------------------------------------------
c_blue "🏷️  Checking tag ${PTAG}…"
git fetch --quiet --tags origin 2>/dev/null || unknown "git fetch --tags origin failed."
if git rev-parse -q --verify "refs/tags/${PTAG}" >/dev/null; then
  c_green "   Tag ${PTAG} exists."
else
  refuse "Tag ${PTAG} does not exist (after git fetch --tags). Only a version cut with ./release.sh can be promoted."
fi

# ---- 2. the build is on R2 ----------------------------------------------------
c_blue "☁️  Checking the build on ${SITE_BASE}/releases/${P}/…"
DMG_BYTES=""; TGZ_BYTES=""; SIG_ON_R2=0
for name in "$PDMG" Nixon.app.tar.gz Nixon.app.tar.gz.sig; do
  r2_head "$name"
  case "$HEAD_CODE" in
    200)
      if [ -n "$HEAD_LEN" ] && [ "$HEAD_LEN" -gt 0 ]; then
        c_green "   ${name}: ${HEAD_LEN} bytes"
        case "$name" in
          "$PDMG") DMG_BYTES="$HEAD_LEN" ;;
          Nixon.app.tar.gz) TGZ_BYTES="$HEAD_LEN" ;;
          *) SIG_ON_R2=1 ;;
        esac
      else
        refuse "releases/${P}/${name} is on R2 but empty (Content-Length ${HEAD_LEN:-missing})."
      fi ;;
    404) refuse "releases/${P}/${name} is not on R2 (HTTP 404). Was ${PTAG} released with ./release.sh --early?" ;;
    *) unknown "Could not HEAD ${SITE_BASE}/releases/${P}/${name} (HTTP ${HEAD_CODE})." ;;
  esac
done

# ---- 3. it is newer than everyone's -------------------------------------------
c_blue "📡 Reading the live feeds at ${SITE_BASE}…"
live_feed_version "updates/latest.json"
case "$LIVE_STATE" in
  ok)
    LIVE_LATEST="$LIVE_VER"
    c_green "   Live updates/latest.json: ${LIVE_LATEST}"
    feed newer "$P" "$LIVE_LATEST" \
      || refuse "${P} is not newer than the everyone feed (updates/latest.json serves ${LIVE_LATEST}). A feed only moves forward, so a version is promoted at most once."
    ;;
  *)
    # Missing included: a 503 can be transient, and promoting blind could move latest.json back.
    LIVE_LATEST="<unknown>"
    unknown "Could not read ${SITE_BASE}/updates/latest.json, so ${P} can't be checked against it."
    ;;
esac
read_live "updates/early.json" "needed for the early feed's forward-only guard"
LIVE_EARLY="$LIVE_VER"

# ---- 4. the signature matches -----------------------------------------------
c_blue "🔏 Checking the signature…"
R2_SIG="${PWORK}/Nixon.app.tar.gz.sig"
UPDATER_PUBKEY="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["plugins"]["updater"]["pubkey"])' "$TAURI_CONF" 2>/dev/null || true)"
[ -n "$UPDATER_PUBKEY" ] || die "plugins.updater.pubkey missing from ${TAURI_CONF}."
if [ "$SIG_ON_R2" -eq 1 ]; then
  if curl -fsS "${CURL_RETRY[@]}" -o "$R2_SIG" "${SITE_BASE}/releases/${P}/Nixon.app.tar.gz.sig"; then
    keyid_status=0
    feed keyid-match --pubkey "$UPDATER_PUBKEY" --sig "$R2_SIG" 2>/dev/null || keyid_status=$?
    case "$keyid_status" in
      0) c_green "   Signature key id matches plugins.updater.pubkey." ;;
      1) refuse "releases/${P}/Nixon.app.tar.gz.sig was made with a different key than plugins.updater.pubkey; installed apps would reject this update." ;;
      *) refuse "releases/${P}/Nixon.app.tar.gz.sig (or plugins.updater.pubkey) is not a minisign blob, so its key id can't be checked." ;;
    esac
  else
    rm -f "$R2_SIG"
    unknown "Could not download ${SITE_BASE}/releases/${P}/Nixon.app.tar.gz.sig."
  fi
fi

# ---- 5. the GitHub pre-release exists (and carries the same build) ------------
if [ "$ALSO_GITHUB" -eq 1 ]; then
  c_blue "🐙 Checking the GitHub pre-release ${PTAG}…"
  gh_err="${PWORK}/gh.err"
  if gh_info="$(gh release view "$PTAG" --json isPrerelease,isDraft,assets \
      --jq '[.isPrerelease, .isDraft, ([.assets[] | select(.name == "Nixon.app.tar.gz") | .size] | first // 0)] | @tsv' 2>"$gh_err")"; then
    IFS=$'\t' read -r gh_pre gh_draft gh_tgz <<<"$gh_info"
    if [ "$gh_draft" = "true" ]; then
      refuse "GitHub release ${PTAG} is a draft; promote needs a published pre-release from ./release.sh --early."
    elif [ "$gh_pre" != "true" ]; then
      refuse "GitHub release ${PTAG} is not a pre-release (it is already a full release, or was not cut with --early)."
    else
      c_green "   ${PTAG} is a published pre-release."
    fi
    if [ -n "$TGZ_BYTES" ] && [ "$gh_tgz" != "$TGZ_BYTES" ]; then
      refuse "The R2 tarball (${TGZ_BYTES} bytes) is not the GitHub pre-release's Nixon.app.tar.gz (${gh_tgz} bytes)."
    fi
    if [ -s "$R2_SIG" ]; then
      if gh release download "$PTAG" -p Nixon.app.tar.gz.sig -D "${PWORK}/gh" >/dev/null 2>"$gh_err"; then
        if cmp -s "$R2_SIG" "${PWORK}/gh/Nixon.app.tar.gz.sig"; then
          c_green "   The R2 signature is byte-identical to the GitHub asset."
        else
          refuse "releases/${P}/Nixon.app.tar.gz.sig on R2 differs from the GitHub pre-release's Nixon.app.tar.gz.sig."
        fi
      elif grep -qi 'no assets\|not found' "$gh_err"; then
        refuse "The GitHub pre-release ${PTAG} has no Nixon.app.tar.gz.sig asset."
      else
        unknown "Could not download Nixon.app.tar.gz.sig from GitHub release ${PTAG} ($(head -1 "$gh_err"))."
      fi
    fi
  elif grep -qi 'not found' "$gh_err"; then
    refuse "There is no GitHub release for ${PTAG}; promote needs the pre-release ./release.sh --early creates (or pass --no-github)."
  else
    unknown "gh release view ${PTAG} failed ($(head -1 "$gh_err"))."
  fi
else
  c_yellow "   --no-github: not checking the GitHub pre-release."
fi

# ---- full cryptographic check, when minisign is available --------------------
if command -v minisign >/dev/null 2>&1 && [ -s "$R2_SIG" ] && [ ${#REFUSALS[@]} -eq 0 ]; then
  c_blue "🔐 Verifying the tarball with minisign…"
  pub_line="$(printf '%s' "$UPDATER_PUBKEY" | base64 -D 2>/dev/null | sed -n 2p)"
  printf '%s' "$(cat "$R2_SIG")" | base64 -D > "${PWORK}/tgz.minisig" 2>/dev/null || true
  if curl -fsS "${CURL_RETRY[@]}" --max-time 600 -o "${PWORK}/Nixon.app.tar.gz" "${SITE_BASE}/releases/${P}/Nixon.app.tar.gz"; then
    if minisign -Vqm "${PWORK}/Nixon.app.tar.gz" -x "${PWORK}/tgz.minisig" -P "$pub_line" >/dev/null 2>&1; then
      c_green "   minisign: the R2 tarball verifies against plugins.updater.pubkey."
    else
      refuse "minisign could not verify releases/${P}/Nixon.app.tar.gz against plugins.updater.pubkey."
    fi
  else
    unknown "Could not download ${SITE_BASE}/releases/${P}/Nixon.app.tar.gz for the minisign check."
  fi
  rm -f "${PWORK}/Nixon.app.tar.gz"
else
  c_yellow "   minisign not on PATH (or an earlier check failed): skipped the full cryptographic check. The key-id check and the GitHub byte comparison still apply, and installed apps verify every payload themselves."
fi

if [ ${#REFUSALS[@]} -gt 0 ]; then
  msg="Refusing to promote ${PTAG}. Nothing has been changed:"
  for r in "${REFUSALS[@]}"; do msg="${msg}
   - ${r}"; done
  die "$msg"
fi
c_green "   ✅ Preflight passed for ${PTAG}."

# ---- publish 1: manifests -----------------------------------------------------
# Notes come from the changelog AT THE TAG, so they are reproducible and unaffected by
# [Unreleased] work that landed since. "Newer than" means newer than live latest.json.
PCHANGELOG="${PWORK}/CHANGELOG.md"
if git rev-parse -q --verify "refs/tags/${PTAG}" >/dev/null; then
  git show "${PTAG}:CHANGELOG.md" > "$PCHANGELOG"
else
  : > "$PCHANGELOG" # dry run with an unknown tag: notes fall back to "Nixon vX"
fi
NOTES_LATEST="${PWORK}/notes-latest.md"
NOTES_EARLY="${PWORK}/notes-early.md"
feed notes --changelog "$PCHANGELOG" --upto "$P" --after "$LIVE_LATEST" --out "$NOTES_LATEST" \
  || die "Could not build the release notes from ${PTAG}:CHANGELOG.md."
feed notes --changelog "$PCHANGELOG" --upto "$P" --after "$LIVE_EARLY" --out "$NOTES_EARLY" \
  || die "Could not build the release notes from ${PTAG}:CHANGELOG.md."

dry=()
[ "$DRY_RUN" -eq 1 ] && dry=( --dry-run )
MANIFEST_GH="${PWORK}/latest.json"         # GitHub asset name must stay latest.json
MANIFEST_R2="${PWORK}/latest.r2.json"
MANIFEST_EARLY="${PWORK}/early.r2.json"
feed manifest --version "$P" --notes "$NOTES_LATEST" --sig "$R2_SIG" --out "$MANIFEST_GH" \
  --url "https://github.com/sxates/nixon/releases/download/${PTAG}/Nixon.app.tar.gz" ${dry[@]+"${dry[@]}"} \
  || die "Could not build the GitHub latest.json."
feed manifest --version "$P" --notes "$NOTES_LATEST" --sig "$R2_SIG" --out "$MANIFEST_R2" \
  --url "${SITE_BASE}/releases/${P}/Nixon.app.tar.gz" ${dry[@]+"${dry[@]}"} \
  || die "Could not build updates/latest.json."
EARLY_WRITE=0
if feed newer "$P" "$LIVE_EARLY"; then
  # † early.json always advertises a version at least as new as latest.json; only a
  # hand-edited bucket gets here.
  EARLY_WRITE=1
  feed manifest --version "$P" --notes "$NOTES_EARLY" --sig "$R2_SIG" --out "$MANIFEST_EARLY" \
    --url "${SITE_BASE}/releases/${P}/Nixon.app.tar.gz" ${dry[@]+"${dry[@]}"} \
    || die "Could not build updates/early.json."
else
  c_yellow "   updates/early.json already advertises ${LIVE_EARLY} (≥ ${P}); it will not be rewritten."
fi
CURRENT_JSON="${PWORK}/current.json"
printf '{"version":"%s","dmg":"releases/%s/%s"}\n' "$P" "$P" "$PDMG" > "$CURRENT_JSON"

# ---- publish 2: R2, latest.json LAST -----------------------------------------
R2_PLAN=( "updates/current.json|${CURRENT_JSON}|application/json" )
[ "$EARLY_WRITE" -eq 1 ] && R2_PLAN+=( "updates/early.json|${MANIFEST_EARLY}|application/json" )
R2_PLAN+=( "updates/latest.json|${MANIFEST_R2}|application/json" )

GH_UPLOAD="gh release upload '$PTAG' '$MANIFEST_GH' --clobber"
GH_EDIT="gh release edit '$PTAG' --prerelease=false --latest --notes-file '$NOTES_LATEST'"
if [ "$ALSO_GITHUB" -eq 1 ]; then GH_STEPS="     ${GH_UPLOAD}
     ${GH_EDIT}"; else GH_STEPS="     (--no-github: no GitHub steps)"; fi
R2_FAIL_STATE="nothing was rebuilt or committed; the uploads listed before this one succeeded; everyone's feed is unchanged unless updates/latest.json was already uploaded. Until it is, re-running ./release.sh promote ${P} is safe and finishes the job."
R2_FAIL_LAST="latest.json"
R2_FAIL_NEXT="the site release.json and the GitHub steps"
R2_FAIL_TAIL="     update ${NIXON_SITE_DIR}/public/release.json (version ${P}, size ${DMG_BYTES:-<dmg bytes>})
${GH_STEPS}"

if [ "$ASSUME_YES" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  printf '\nAbout to advertise %s to EVERYONE (updates/latest.json) and make it the latest GitHub release. Continue? [y/N] ' "$PTAG"
  read -r ans || true
  case "$ans" in y|Y|yes|YES) ;; *) die "Aborted before any upload. Nothing has been changed." ;; esac
fi

r2_upload_plan

# ---- publish 3: verify the live feed -------------------------------------------
if [ "$DRY_RUN" -eq 0 ]; then
  verify_die() {
    die "$1
   updates/latest.json was already uploaded, so a re-run of promote will refuse (${P} is no longer newer).
   Skipped because of this: the site release.json push and the GitHub steps.
   To finish: re-check ${SITE_BASE}/updates/latest.json, then update ${NIXON_SITE_DIR}/public/release.json (version ${P}, size ${DMG_BYTES}) and run:
${GH_STEPS}"
  }
  verify_live "updates/latest.json" "$P" "$PDMG" "$DMG_BYTES" "$TGZ_BYTES"
fi

# ---- publish 4: site release.json (warn, never fail) ---------------------------
if [ "$DRY_RUN" -eq 1 ]; then
  echo "  [dry-run] write ${NIXON_SITE_DIR}/public/release.json (version ${P}, size ${DMG_BYTES:-<unknown>}) and push"
else
  publish_release_json "$P" "$DMG_BYTES" || c_yellow "$RELEASE_JSON_WARN"
fi

# ---- publish 5: GitHub — legacy manifest first, then the pre-release flip ------
if [ "$ALSO_GITHUB" -eq 1 ]; then
  c_blue "🚀 Promoting the GitHub release ${PTAG}…"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "  [dry-run] ${GH_UPLOAD}"
    echo "  [dry-run] ${GH_EDIT}"
  else
    gh release upload "$PTAG" "$MANIFEST_GH" --clobber \
      || die "gh release upload failed. Everyone's feed (updates/latest.json) already serves ${P}. Finish with:
${GH_STEPS}"
    # The flip comes last: it is what makes the build visible to the legacy GitHub endpoint.
    gh release edit "$PTAG" --prerelease=false --latest --notes-file "$NOTES_LATEST" \
      || die "gh release edit failed. Everyone's feed already serves ${P} and the GitHub latest.json is uploaded. Finish with:
     ${GH_EDIT}"
  fi
else
  c_yellow "⏭️  --no-github: the GitHub release stays a pre-release. Installs still on the GitHub endpoint will not see ${P}."
fi

if [ "$DRY_RUN" -eq 1 ]; then
  echo "  [dry-run] latest.json (GitHub):"; sed 's/^/    /' "$MANIFEST_GH"; echo
  echo "  [dry-run] updates/latest.json (R2):"; sed 's/^/    /' "$MANIFEST_R2"; echo
  if [ "$EARLY_WRITE" -eq 1 ]; then
    echo "  [dry-run] updates/early.json (R2):"; sed 's/^/    /' "$MANIFEST_EARLY"; echo
  fi
fi

c_green "✅ Promoted ${PTAG} to everyone. Nothing was rebuilt."
if [ "$DRY_RUN" -eq 0 ]; then
  echo "   Installed apps will pick this update up on their own (in-app updater, specs/0058)."
  echo "   Download page: ${SITE_BASE}/download/"
fi
