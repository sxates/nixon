import Sidebar from '@/components/Sidebar'
import MainContent from '@/components/MainContent'
import { OnboardingFlow } from '@/components/onboarding'
import { DownloadProgressToastProvider } from '@/components/shared/DownloadProgressToast'
import CommandPalette from '@/components/CommandPalette'
import { LlmActivityProvider } from '@/contexts/LlmActivityProvider'
import { TransportRail } from '@/components/Transport/TransportRail'
import ResumeRecordingPrompt from '@/components/ResumeRecordingPrompt'
import ZoomAutoDetect from '@/components/ZoomAutoDetect'
import NotificationPermissionBootstrap from '@/components/NotificationPermissionBootstrap'
import CalendarAlerts from '@/components/Calendar/CalendarAlerts'
import VoiceprintRetractionListener from '@/components/People/VoiceprintRetractionListener'
import PermissionsModal from '@/components/PermissionsModal'

interface AppShellProps {
  showOnboarding: boolean
  onOnboardingComplete: () => void
  children: React.ReactNode
}

// specs/0061 W1 Task 2 — extracted from RootLayout's showOnboarding ternary so the
// download-progress toast provider (top-right toasts) only mounts post-onboarding.
// The onboarding DownloadProgressStep already renders its own in-page progress
// cards, so mounting the toast provider during onboarding duplicated that
// feedback (first external user report).
export function AppShell({ showOnboarding, onOnboardingComplete, children }: AppShellProps) {
  if (showOnboarding) {
    return <OnboardingFlow onComplete={onOnboardingComplete} />
  }

  return (
    <div className="flex">
      {/* Download progress toast provider - listens for background downloads.
          Post-onboarding only (specs/0061 W1) — during onboarding the download
          steps show their own progress cards. */}
      <DownloadProgressToastProvider />

      {/* Scoped to the Sidebar and the transport rail deliberately
          (specs/0052 + 0057): they are the only two consumers — the rail's
          queue is the second (decision 8) — and every llm-activity-changed
          event sets state here. Hoisting it above MainContent would
          re-render the whole page tree on each background task transition
          — a prep pass emits a burst of them. */}
      <LlmActivityProvider>
        <Sidebar />
        {/* specs/0057 decision 7 — THE transport: fixed bottom rail on
            every post-onboarding route, with the deck status, the REC/HOLD/
            STOP keys and the one global queue. Replaces GlobalRecordingBar
            and the deferred-backlog pill. */}
        <TransportRail />
      </LlmActivityProvider>
      <MainContent>{children}</MainContent>
      {/* ⌘K command palette — global, every route (post-onboarding) */}
      <CommandPalette />
      {/* Request OS notification permission up front (post-onboarding) */}
      <NotificationPermissionBootstrap />
      {/* Zoom auto-detection — global listeners for record/stop (post-onboarding) */}
      <ZoomAutoDetect />
      {/* Calendar "time to join" alerts — app-wide, fires before meetings (spec 0008) */}
      <CalendarAlerts />
      {/* Voiceprint retraction feedback — app-wide undo toast when a span
          correction quarantines a person's polluted voice samples (spec 0039 WS3) */}
      <VoiceprintRetractionListener />
      {/* "Enable recording" permissions modal — opened from the sidebar
          Permissions nav item (spec 0014) */}
      <PermissionsModal />
      {/* Relaunch recovery — prompt to resume a crash-interrupted
          recording, one at a time (spec 0037) */}
      <ResumeRecordingPrompt />
    </div>
  )
}
