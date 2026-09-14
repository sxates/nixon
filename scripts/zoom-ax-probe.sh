#!/usr/bin/env bash
# specs/0049 Task 2 — Zoom mute-state accessibility spike.
#
# Dumps the mute-related bits of the Zoom desktop app's macOS Accessibility (AX)
# tree, using the built-in System Events (same AX API the Rust reader will use —
# no dependency, no build). Run it DURING a live Zoom meeting, once while MUTED
# and once while UNMUTED, and diff the two outputs. Whatever attribute flips is
# the stable signal the Rust reader (Task 3) should read.
#
# Usage:
#   1. Join a Zoom meeting.
#   2. System Settings → Privacy & Security → Accessibility → enable your terminal
#      app (Terminal / iTerm / VS Code). Without this, System Events returns
#      error -1743 ("not allowed assistive access").
#   3. Mute yourself in Zoom, then:   ./scripts/zoom-ax-probe.sh > muted.txt
#   4. Unmute yourself, then:         ./scripts/zoom-ax-probe.sh > unmuted.txt
#   5. diff muted.txt unmuted.txt   (or paste both back here)
#
# The filter matches English "Mute/Unmute/Audio/Microphone"; on a localized Zoom,
# tell me your language and I'll widen it.
set -euo pipefail

echo "# zoom-ax-probe  (run muted, then unmuted, then diff)"
echo "# host: $(scutil --get ComputerName 2>/dev/null || echo '?')   zoom running: $(pgrep -xq 'zoom.us' && echo yes || echo no)"
echo

osascript <<'APPLESCRIPT'
on trimJoin(a, b, c)
  return (a as text) & " | " & (b as text) & " | " & (c as text)
end trimJoin

on run
  set report to ""
  tell application "System Events"
    if not (exists process "zoom.us") then
      return "Zoom (process 'zoom.us') is not running. Start/join a Zoom meeting, then re-run."
    end if
    tell process "zoom.us"

      -- 1) Menu bar → Meeting menu: Zoom usually exposes a 'Mute Audio' /
      --    'Unmute Audio' item here whose title flips with state.
      set report to report & "== MENU BAR ITEMS ==" & linefeed
      try
        repeat with mbi in menu bar items of menu bar 1
          try
            set report to report & "  " & (name of mbi as text) & linefeed
          end try
        end repeat
      end try

      set report to report & linefeed & "== MENU ITEMS mentioning mute/audio/mic ==" & linefeed
      try
        repeat with mbi in menu bar items of menu bar 1
          try
            repeat with mi in menu items of menu 1 of mbi
              set nm to ""
              try
                set nm to (name of mi as text)
              end try
              if nm contains "ute" or nm contains "udio" or nm contains "icrophone" then
                set mark to ""
                try
                  set mark to (value of attribute "AXMenuItemMarkChar" of mi as text)
                end try
                set report to report & "  menuitem: '" & nm & "'  mark='" & mark & "'  (under '" & (name of mbi as text) & "')" & linefeed
              end if
            end repeat
          end try
        end repeat
      end try

      -- (Removed the `entire contents of window` walk — it hung on Zoom's huge
      --  meeting window. The signal we use lives in the Meeting menu above:
      --  a self-mic item titled "Mute audio" (unmuted) / "Unmute audio" (muted),
      --  distinct from the host "Mute all" / "Ask all to unmute" items.)
      set report to report & linefeed & "== WINDOW TITLES ==" & linefeed
      repeat with w in windows
        try
          set report to report & "  WINDOW: '" & (name of w as text) & "'" & linefeed
        end try
      end repeat

    end tell
  end tell
  return report
end run
APPLESCRIPT
