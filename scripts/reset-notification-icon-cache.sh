#!/usr/bin/env bash
#
# reset-notification-icon-cache.sh — one-time fix for the stale Meetily icon on
# Nixon's macOS notifications (specs/0041 WS8; specs/BACKLOG 2026-07-08).
#
# Why this exists: a macOS notification wears the icon that Notification Center
# has CACHED for the sending bundle id (ai.vinyl.app) — not the icon inside the
# installed .app. Early Nixon builds shipped under ai.vinyl.app while icons/
# still held Meetily artwork, seeding that cache; reinstalling a correctly-iconed
# bundle never evicts it. The Rust side can't override it either: the notification
# plugin delegates to notify-rust, which documents that macOS ignores
# per-notification icons (see notifications/system.rs).
#
# What this does (safe, idempotent, touches no app data):
#   1. Force LaunchServices to re-register /Applications/Nixon.app so the
#      system's icon record for ai.vinyl.app points at the current artwork.
#   2. Restart the Notification Center processes so they drop their cached
#      per-bundle-id icon attribution. launchd relaunches them immediately;
#      pending notifications are unaffected. On some macOS versions `usernoted`
#      runs under a different name or not at all — a miss there is fine.
#
# FALLBACK: if a fresh notification still shows the Meetily icon after running
# this, log out and back in (or reboot) — that rebuilds the per-session caches.
set -euo pipefail

c_blue()  { printf '\033[0;34m%s\033[0m\n' "$*"; }
c_green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
c_yellow() { printf '\033[1;33m%s\033[0m\n' "$*"; }
die() { printf '\033[0;31mERROR: %s\033[0m\n' "$*" >&2; exit 1; }

APP="/Applications/Nixon.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

# ---- guards -----------------------------------------------------------------
[[ "$(uname -s)" == "Darwin" ]] || die "This script is macOS-only (Notification Center icon cache)."
[[ -d "$APP" ]] || die "$APP not found. Install the production app first (frontend/upgrade-nixon.sh), then re-run."
[[ -x "$LSREGISTER" ]] || die "lsregister not found at the expected path — macOS layout changed? ($LSREGISTER)"

# ---- 1. re-register the app with LaunchServices ------------------------------
c_blue "1/2 Re-registering $APP with LaunchServices (refreshes the icon record)..."
"$LSREGISTER" -f "$APP"
c_green "    Re-registered."

# ---- 2. restart the Notification Center processes ----------------------------
# NotificationCenter = the UI; usernoted = the user notification daemon. Both
# respawn via launchd within a second. Process names vary across macOS versions
# (usernoted may be absent on recent releases), so a miss is non-fatal.
c_blue "2/2 Restarting Notification Center processes to drop the cached icon..."
restarted=0
for proc in NotificationCenter usernoted; do
  if killall "$proc" 2>/dev/null; then
    c_green "    Restarted $proc."
    restarted=1
  else
    c_yellow "    $proc not running (or not present on this macOS version) — skipped."
  fi
done
if [[ "$restarted" -eq 0 ]]; then
  c_yellow "    No notification process was restarted; the cache may only clear on logout/reboot."
fi

# ---- verify -------------------------------------------------------------------
echo
c_green "Done. Verify:"
echo "  1. Open Nixon.app and start a recording (or trigger any notification,"
echo "     e.g. Settings -> test notification if available)."
echo "  2. The 'Recording started' banner should show the NIXON icon."
echo "  3. Still Meetily? Log out and back in (or reboot), then check again —"
echo "     the per-session icon caches rebuild on login."
