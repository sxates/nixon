#!/bin/bash
# Build + install "Dev Nixon.app" — a bundled, isolated DEV app.
#
# Why: the bare `./dev-nixon.sh` binary can't get macOS Audio Capture / Notifications /
# Calendar TCC grants (no Info.plist, ad-hoc signature churns every rebuild), so OS-integration
# features (recording system audio, actionable notifications, calendar) can't be tested there.
# This produces a real .app bundle under the DEV identifier `ai.vinyl.app.debug` ("Dev Nixon"),
# which macOS CAN grant those permissions — while keeping data isolated from production Nixon.app
# (ai.vinyl.app). Grant permissions to Dev Nixon once; rebuild here to pick up code changes.
#
# Tradeoff: no hot-reload (it's a release bundle). Use `./dev-nixon.sh` for fast code iteration,
# this for testing the real recording/Zoom/calendar experience.
set -euo pipefail

source "$HOME/.cargo/env" 2>/dev/null || true
cd "$(dirname "$0")"   # frontend/

if ! command -v cargo >/dev/null 2>&1; then
  echo "❌ cargo not found. Install Rust: https://rustup.rs" >&2
  exit 1
fi

APP_SRC="../target/release/bundle/macos/Dev Nixon.app"

echo "🔨 Building Dev Nixon.app (release, Metal, identifier ai.vinyl.app.debug)..."
# NIXON_DEV_BUNDLE=1 makes scripts/tauri-auto.js merge src-tauri/tauri.dev.conf.json into the
# production build, so the bundle is "Dev Nixon" / ai.vinyl.app.debug instead of prod Nixon.
NIXON_DEV_BUNDLE=1 ./build-gpu.sh

[ -d "$APP_SRC" ] || { echo "❌ Build did not produce $APP_SRC"; exit 1; }

# One-time legacy cleanup (specs/0057 rename): before 0057 the dev bundle installed as
# /Applications/Dev Vinyl.app. Leaving it behind would put two bundles on disk sharing the SAME
# bundle id ai.vinyl.app.debug — so the same dev SQLite DB, the same single-instance lock, and the
# same TCC (mic/audio-capture/notification) grants. Dev data lives under the bundle IDENTIFIER,
# not the .app, so removing the old bundle loses nothing. Idempotent once it's gone.
if [ -d "/Applications/Dev Vinyl.app" ]; then
  echo "🧹 Removing the pre-rename /Applications/Dev Vinyl.app (same bundle id; dev data is identifier-keyed and untouched)..."
  osascript -e 'tell application "Dev Vinyl" to quit' 2>/dev/null || true
  pkill -f "/Applications/Dev Vinyl.app/Contents/MacOS" 2>/dev/null || true
  sleep 2
  rm -rf "/Applications/Dev Vinyl.app"
fi

echo "🛑 Quitting any running Dev Nixon + dev server..."
osascript -e 'tell application "Dev Nixon" to quit' 2>/dev/null || true
pkill -f "/Applications/Dev Nixon.app/Contents/MacOS" 2>/dev/null || true
pkill -f "target/debug/nixon" 2>/dev/null || true         # the bare dev binary (same identifier)
pkill -f "next dev -p 3118" 2>/dev/null || true
sleep 2

echo "📦 Installing to /Applications (dev data in ~/Library/Application Support/ai.vinyl.app.debug is preserved)..."
rm -rf "/Applications/Dev Nixon.app"
cp -R "$APP_SRC" "/Applications/"
xattr -cr "/Applications/Dev Nixon.app" 2>/dev/null || true

echo "✅ Built. Launching Dev Nixon — grant Audio Capture / Notifications / Calendar when prompted."
echo "   (Separate app + data from production Nixon.app; permissions are granted independently.)"
open "/Applications/Dev Nixon.app"
