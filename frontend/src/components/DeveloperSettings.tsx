"use client"

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Button } from "./ui/button"
import { Switch } from "./ui/switch"
import { SettingsGroup, SettingsNote, SettingsRow, SettingsSection } from "./ui/settings"
import { CATEGORY_MEETING, CATEGORY_PREP, notify } from "@/lib/osNotification"
import { joinAndRecord } from "@/lib/calendar"
import { useRecordingState } from "@/contexts/RecordingStateContext"
import { useSidebar } from "@/components/Sidebar/SidebarProvider"

export interface DevFlags { fixtures: boolean; no_audio: boolean; fake_downloads: boolean; reset_onboarding: boolean; control: boolean }
interface SeedReport { meetings: number; people: number; segments: number; failed: number }

export function formatFlags(f: DevFlags): string {
  const parts = [f.fixtures && "fixtures=demo", f.no_audio && "no-audio", f.fake_downloads && "fake-downloads", f.reset_onboarding && "reset-onboarding", f.control && "control"].filter(Boolean)
  return parts.length ? parts.join(" · ") : "none"
}

function formatReport(r: SeedReport): string {
  const base = `${r.meetings} meetings, ${r.people} people, ${r.segments} segments`
  return r.failed > 0 ? `${base}, ${r.failed} failed` : base
}

/** specs/0059 — rendered only when the debug-only `dev_get_flags` command exists. */
export function DeveloperSettings() {
  const { isRecording } = useRecordingState()
  const { handleRecordingToggle } = useSidebar()
  const [flags, setFlags] = useState<DevFlags | null>(null)
  const [skipAudio, setSkipAudio] = useState(false)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    invoke<DevFlags>("dev_get_flags").then(setFlags).catch(() => setFlags(null))
  }, [])

  if (!flags) return null

  // specs/0059 fix round 2: reload after a successful load so the sidebar and Today view
  // (already-mounted providers holding stale meeting/people lists) refetch against the
  // newly-seeded data. The report goes in the toast description, not component state,
  // because this section unmounts on reload before a state update could ever render.
  const load = async () => {
    setBusy(true)
    try {
      const r = await invoke<SeedReport>("dev_load_fixtures", { noAudio: skipAudio })
      toast.success("Demo data loaded", { description: formatReport(r) })
      window.location.reload()
    } catch (e) {
      toast.error(`Could not load demo data: ${String(e)}`)
    } finally { setBusy(false) }
  }

  // specs/0068 — the only way to exercise the action buttons without waiting for a real
  // meeting. Dev-only on purpose: it is a probe, not a feature.
  const testAlert = async (starting: boolean) => {
    const sent = await notify({
      title: starting ? "Standup starting now" : "Standup in 5 min",
      body: "10:00 · Work",
      category: starting ? CATEGORY_MEETING : CATEGORY_PREP,
      id: "dev-test-meeting",
      onPrep: () => toast.success("Prep pressed", { description: "The delegate routed the press back into Nixon." }),
      // The REAL path, not a toast: this is the half that has to work without the user
      // touching Nixon, so the probe has to exercise it rather than stand in for it.
      // No zoomUrl, so nothing is launched — only the recording half runs.
      onJoinAndRecord: () =>
        void joinAndRecord(
          { id: `dev-probe-${Date.now()}`, title: "Dev probe meeting", zoomUrl: null, startsAt: new Date().toISOString() },
          isRecording,
          handleRecordingToggle,
        ),
      onOpen: () => toast.success("Banner tapped", { description: "Default action routed back into Nixon." }),
    })
    if (!sent) toast.error("Could not send the alert", { description: "Check Settings → General → Notifications." })
  }

  const reset = async () => {
    try { await invoke("dev_reset_onboarding") } catch (e) { toast.error(`Could not reset onboarding: ${String(e)}`) }
  }

  return (
    <SettingsSection title="Developer" description="Only in the development build. Data lives in the isolated debug profile.">
      <SettingsGroup>
        <SettingsRow label="Active dev flags" description={formatFlags(flags)} />
        <SettingsRow
          label="Load demo data"
          description="Replaces every meeting in the debug profile with the fictional dataset, then reloads."
          control={
            <div className="flex items-center gap-3">
              <label className="flex items-center gap-2 text-xs text-muted-foreground">
                <Switch checked={skipAudio} onCheckedChange={setSkipAudio} aria-label="Skip audio synthesis" /> skip audio
              </label>
              <Button size="sm" variant="secondary" disabled={busy} onClick={load}>{busy ? "Loading…" : "Load demo data"}</Button>
            </div>
          }
        />
        <SettingsRow
          label="Test the meeting alerts"
          description="Fires the real banners. Put another window in front first. Prep and the body tap toast here; Starting now runs the REAL Join & Record path (no Zoom link, so only the recording half) — it will start an actual recording in this debug profile."
          control={
            <div className="flex items-center gap-2">
              <Button size="sm" variant="secondary" onClick={() => testAlert(false)}>
                T-5 (Prep)
              </Button>
              <Button size="sm" variant="secondary" onClick={() => testAlert(true)}>
                Starting now
              </Button>
            </div>
          }
        />
        <SettingsRow
          label="Reset onboarding and restart"
          description="Clears the onboarding status. The app reloads on the Welcome step."
          control={<Button size="sm" variant="secondary" onClick={reset}>Reset onboarding</Button>}
        />
      </SettingsGroup>
      <SettingsNote tone="muted">Launch flags: ./dev-nixon.sh --demo, --no-audio, --onboarding, --real-downloads, --control</SettingsNote>
    </SettingsSection>
  )
}
