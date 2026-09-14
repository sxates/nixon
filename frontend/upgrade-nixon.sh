#!/bin/bash
# Upgrade the installed /Applications/Nixon.app from a fresh production build.
#
# YOUR DATA IS PRESERVED. Meetings/notes/settings/models live in
#   ~/Library/Application Support/ai.vinyl.app/   (meeting_minutes.sqlite + *.json + models)
#   ~/Movies/nixon-recordings/ — or the legacy ~/Movies/meetily-recordings/ if you already have it (audio)
# These are keyed by the bundle id, NOT the .app bundle, so replacing the app never touches them.
# (On the first launch after the com.vinyl.dev -> ai.vinyl.app rebrand, the app copies your data
#  from the old com.vinyl.dev dir into ai.vinyl.app automatically; see src/data_migration.rs.)
# DB migrations are forward-only/additive, so schema upgrades keep existing rows.
#
# Caveat: dev builds are ad-hoc signed ("-"), so the signature changes every build and macOS may
# ask you to re-grant Screen Recording on first launch after an upgrade (one click; no data impact).
set -euo pipefail

source "$HOME/.cargo/env" 2>/dev/null || true
cd "$(dirname "$0")"   # frontend/

DB="$HOME/Library/Application Support/ai.vinyl.app/meeting_minutes.sqlite"
APP_SRC="../target/release/bundle/macos/Nixon.app"

# Safety net: timestamped backup of the DB before we touch anything.
if [ -f "$DB" ]; then
  BACKUP="$DB.backup-$(date +%Y%m%d-%H%M%S)"
  cp "$DB" "$BACKUP"
  echo "🛟 Backed up database -> $BACKUP"
fi

# Google Calendar OAuth client (optional, specs/0032): compile-time values;
# without them the Google provider is inert. Same file dev-nixon.sh sources.
if [ -f "src-tauri/.env.google" ]; then
  set -a
  . "src-tauri/.env.google"
  set +a
fi

# specs/0057 rename guard: pre-0057 .env.google files export VINYL_GOOGLE_CLIENT_ID/_SECRET,
# which the build no longer reads — Calendar would go silently inert. Warn, don't abort.
if [ -f "src-tauri/.env.google" ] && { [ -z "${NIXON_GOOGLE_CLIENT_ID:-}" ] || [ -z "${NIXON_GOOGLE_CLIENT_SECRET:-}" ]; }; then
  {
    echo ""
    echo "⚠️  Google Calendar will be INERT in this build."
    echo "   src-tauri/.env.google exists but NIXON_GOOGLE_CLIENT_ID/NIXON_GOOGLE_CLIENT_SECRET are not both set."
    echo "   A pre-0057 .env.google still uses the old VINYL_ prefix; rename both keys:"
    echo "     VINYL_GOOGLE_CLIENT_ID     -> NIXON_GOOGLE_CLIENT_ID"
    echo "     VINYL_GOOGLE_CLIENT_SECRET -> NIXON_GOOGLE_CLIENT_SECRET"
    echo "   See SETUP.md \"Google Calendar OAuth client\"."
    echo ""
  } >&2
fi

echo "🔨 Building Nixon.app (release, Metal)..."
./build-gpu.sh

[ -d "$APP_SRC" ] || { echo "❌ Build did not produce $APP_SRC"; exit 1; }

# One-time legacy cleanup (specs/0057 rename): before 0057 this app installed as
# /Applications/Vinyl.app. Leaving it behind would put two bundles on disk sharing the SAME
# bundle id ai.vinyl.app — so the same SQLite DB, the same single-instance lock, and the same
# TCC (mic/screen-recording) grants. Data lives under the bundle IDENTIFIER, not the .app, so
# removing the old bundle loses nothing. Idempotent: a no-op once Vinyl.app is gone.
if [ -d "/Applications/Vinyl.app" ]; then
  echo "🧹 Removing the pre-rename /Applications/Vinyl.app (same bundle id; your data is identifier-keyed and untouched)..."
  osascript -e 'tell application "Vinyl" to quit' 2>/dev/null || true
  pkill -f "/Applications/Vinyl.app/Contents/MacOS" 2>/dev/null || true
  sleep 2
  rm -rf "/Applications/Vinyl.app"
fi

echo "🛑 Quitting running Nixon..."
osascript -e 'tell application "Nixon" to quit' 2>/dev/null || true
pkill -f "/Applications/Nixon.app/Contents/MacOS" 2>/dev/null || true
sleep 2

echo "📦 Installing to /Applications (data preserved)..."
rm -rf "/Applications/Nixon.app"
cp -R "$APP_SRC" "/Applications/"
xattr -cr "/Applications/Nixon.app" 2>/dev/null || true

echo "✅ Upgraded. Your meetings/notes/settings are intact."
echo "   If macOS asks on first launch, re-grant Screen Recording (System Settings > Privacy)."
open "/Applications/Nixon.app"
