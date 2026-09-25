'use client'

import './globals.css'
import { Archivo, Archivo_Narrow, IBM_Plex_Sans, IBM_Plex_Mono, Courier_Prime } from 'next/font/google'
import { SidebarProvider } from '@/components/Sidebar/SidebarProvider'
import { Toaster, toast } from 'sonner'
import "sonner/dist/styles.css"
import { useState, useEffect, useCallback } from 'react'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { safeListen, makeSafeUnlisten } from '@/lib/safe-listen'
import { TooltipProvider } from '@/components/ui/tooltip'
import { RecordingStateProvider } from '@/contexts/RecordingStateContext'
import { UpdateStatusProvider } from '@/contexts/UpdateStatusContext'
import { RestartConfirmProvider } from '@/contexts/RestartConfirmContext'
import { ThemeProvider, useTheme } from '@/contexts/ThemeContext'
import { OllamaDownloadProvider } from '@/contexts/OllamaDownloadContext'
import { TranscriptProvider } from '@/contexts/TranscriptContext'
import { ConfigProvider } from '@/contexts/ConfigContext'
import { OnboardingProvider } from '@/contexts/OnboardingContext'
import { RecordingPostProcessingProvider } from '@/contexts/RecordingPostProcessingProvider'
import { ImportAudioDialog, ImportDropOverlay } from '@/components/ImportAudio'
import { DeferredBacklogProvider } from '@/contexts/DeferredBacklogProvider'
import { QueueOpenProvider } from '@/contexts/QueueOpenContext'
import { ImportDialogProvider } from '@/contexts/ImportDialogContext'
import { PermissionsModalProvider } from '@/contexts/PermissionsModalContext'
import { CalmMotionProvider } from '@/contexts/CalmMotionContext'
import { isAudioExtension, getAudioFormatsDisplayList } from '@/constants/audioFormats'
import { AppShell } from './_components/AppShell'


// specs/0057 — deck typography. Archivo is the panel/UI face (tabular figures for
// counters), Archivo Narrow the meter scales, Plex Sans the reading face, Plex Mono
// timecodes, Courier Prime the typewriter-on-paper transcript body.
const archivo = Archivo({ subsets: ['latin'], weight: ['400', '500', '600', '700'], variable: '--font-archivo' })
const archivoNarrow = Archivo_Narrow({ subsets: ['latin'], weight: ['400'], variable: '--font-archivo-narrow' })
const plexSans = IBM_Plex_Sans({ subsets: ['latin'], weight: ['400', '500', '600'], variable: '--font-plex-sans' })
const plexMono = IBM_Plex_Mono({ subsets: ['latin'], weight: ['400', '500', '700'], variable: '--font-plex-mono' })
const courierPrime = Courier_Prime({ subsets: ['latin'], weight: ['400', '700'], variable: '--font-courier-prime' })

// Module-level component — stable reference across RootLayout re-renders.
// Defined here (not inside RootLayout) so React never sees a new function type
// on re-render, which would cause unmount/remount and break initialization logic.
// It used to gate the dialog on the beta flag; since specs/0066 W3 it only keeps that
// stable identity.
function ConditionalImportDialog({
  showImportDialog,
  handleImportDialogClose,
  importFilePath,
}: {
  showImportDialog: boolean;
  handleImportDialogClose: (open: boolean) => void;
  importFilePath: string | null;
}) {
  return (
    <ImportAudioDialog
      open={showImportDialog}
      onOpenChange={handleImportDialogClose}
      preselectedFile={importFilePath}
    />
  );
}

// export { metadata } from './metadata'

// specs/0057 — Sonner needs the resolved theme explicitly; it can't read our `.dark`
// class. RootLayout itself renders <ThemeProvider>, so useTheme() has to be called from
// a child component rendered inside it.
function ThemedToaster({ offset }: { offset?: { bottom: string } }) {
  const { resolved } = useTheme()
  return (
    <Toaster
      position="bottom-center"
      richColors
      closeButton
      theme={resolved}
      offset={offset}
      mobileOffset={offset}
      toastOptions={{ classNames: { toast: 'rounded-[3px] shadow-sm font-sans' } }}
    />
  )
}

/** specs/0057 — bottom-center toasts would land underneath the fixed transport rail, so they
 *  clear its height plus the normal 16px gutter. Onboarding has no rail, hence no offset.
 *  `--toast-lift` is set only while the transcript's selection bar is up (SelectionActionBar),
 *  so a toast stacks above that bar instead of over it. */
const RAIL_TOAST_OFFSET = { bottom: 'calc(var(--rail-h) + 16px + var(--toast-lift, 0px))' }

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  const [showOnboarding, setShowOnboarding] = useState(false)
  const [_onboardingCompleted, setOnboardingCompleted] = useState(false)

  // Import audio state
  const [showDropOverlay, setShowDropOverlay] = useState(false)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [importFilePath, setImportFilePath] = useState<string | null>(null)

  useEffect(() => {
    // specs/0060: screenshot drivers wait for this attribute. 400 ms clears the
    // 0.25–0.3 s framer intro animations on every page. Set on both outcomes below —
    // a failed status check still renders onboarding.
    let shotReadyTimer: number | undefined
    const markShotReady = () => {
      shotReadyTimer = window.setTimeout(() => { document.documentElement.dataset.shotReady = '1' }, 400)
    }

    // Check onboarding status first
    invoke<{ completed: boolean } | null>('get_onboarding_status')
      .then((status) => {
        const isComplete = status?.completed ?? false
        setOnboardingCompleted(isComplete)

        if (!isComplete) {
          console.log('[Layout] Onboarding not completed, showing onboarding flow')
          setShowOnboarding(true)
        } else {
          console.log('[Layout] Onboarding completed, showing main app')
        }

        markShotReady()
      })
      .catch((error) => {
        console.error('[Layout] Failed to check onboarding status:', error)
        // Default to showing onboarding if we can't check
        setShowOnboarding(true)
        setOnboardingCompleted(false)

        markShotReady()
      })

    return () => { if (shotReadyTimer !== undefined) window.clearTimeout(shotReadyTimer) }
  }, [])

  // Disable context menu in production
  useEffect(() => {
    if (process.env.NODE_ENV === 'production') {
      const handleContextMenu = (e: MouseEvent) => e.preventDefault();
      document.addEventListener('contextmenu', handleContextMenu);
      return () => document.removeEventListener('contextmenu', handleContextMenu);
    }
  }, []);
  useEffect(() => {
    // Listen for tray recording toggle request
    return safeListen('request-recording-toggle', () => {
      console.log('[Layout] Received request-recording-toggle from tray');

      if (showOnboarding) {
        toast.error("Please complete setup first", {
          description: "You need to finish onboarding before you can start recording."
        });
      } else {
        // If in main app, forward to useRecordingStart via window event
        console.log('[Layout] Forwarding to start-recording-from-sidebar');
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      }
    });
  }, [showOnboarding]);

  // specs/0059 — the debug-only `dev_reset_onboarding` command emits this event after
  // clearing the onboarding status; reload so the app re-checks it and shows the Welcome step.
  useEffect(() => {
    return safeListen('onboarding-reset', () => {
      console.log('[Layout] Received onboarding-reset, reloading');
      window.location.reload();
    });
  }, []);

  // Handle file drop for audio import (ungated since specs/0066 W3)
  const handleFileDrop = useCallback((paths: string[]) => {
    // Find the first audio file
    const audioFile = paths.find(p => {
      const ext = p.split('.').pop()?.toLowerCase();
      return !!ext && isAudioExtension(ext);
    });

    if (audioFile) {
      console.log('[Layout] Audio file dropped:', audioFile);
      setImportFilePath(audioFile);
      setShowImportDialog(true);
    } else if (paths.length > 0) {
      toast.error('Please drop an audio file', {
        description: `Supported formats: ${getAudioFormatsDisplayList()}`
      });
    }
  }, []);

  // Listen for drag-drop events
  useEffect(() => {
    if (showOnboarding) return; // Don't handle drops during onboarding

    const unlisteners: UnlistenFn[] = [];
    const cleanedUpRef = { current: false };

    const setupListeners = async () => {
      const unlistenDragEnter = makeSafeUnlisten(await listen('tauri://drag-enter', () => {
        setShowDropOverlay(true);
      }));
      if (cleanedUpRef.current) {
        unlistenDragEnter();
        return;
      }
      unlisteners.push(unlistenDragEnter);

      // Drag leave - hide overlay
      const unlistenDragLeave = makeSafeUnlisten(await listen('tauri://drag-leave', () => {
        setShowDropOverlay(false);
      }));
      if (cleanedUpRef.current) {
        unlistenDragLeave();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDragLeave);

      // Drop - process files
      const unlistenDrop = makeSafeUnlisten(await listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
        setShowDropOverlay(false);
        handleFileDrop(event.payload.paths);
      }));
      if (cleanedUpRef.current) {
        unlistenDrop();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDrop);
    };

    setupListeners();

    return () => {
      cleanedUpRef.current = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [showOnboarding, handleFileDrop]);

  // Handle import dialog close
  const handleImportDialogClose = useCallback((open: boolean) => {
    setShowImportDialog(open);
    if (!open) {
      setImportFilePath(null);
    }
  }, []);

  // Handler for ImportDialogProvider - opens import dialog from any child component
  const handleOpenImportDialog = useCallback((filePath?: string | null) => {
    setImportFilePath(filePath ?? null);
    setShowImportDialog(true);
  }, []);

  const handleOnboardingComplete = () => {
    console.log('[Layout] Onboarding completed, reloading app')
    setShowOnboarding(false)
    setOnboardingCompleted(true)
    // Optionally reload the window to ensure all state is fresh
    window.location.reload()
  }

  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        {/* specs/0057 — apply the theme before first paint (static export has no .dark in the
            prerendered HTML). Mirrors ThemeContext's resolution: stored 'light'|'dark' wins,
            else the OS preference. Keep in sync with THEME_STORAGE_KEY. */}
        <script
          dangerouslySetInnerHTML={{
            __html: `(function(){try{var p=localStorage.getItem('nixon.theme');var d=p==='dark'||(p!=='light'&&window.matchMedia&&window.matchMedia('(prefers-color-scheme: dark)').matches);if(d)document.documentElement.classList.add('dark');}catch(e){}})();`,
          }}
        />
      </head>
      <body className={`${archivo.variable} ${archivoNarrow.variable} ${plexSans.variable} ${plexMono.variable} ${courierPrime.variable} font-sans antialiased`}>
        {/* specs/0057 — outermost so the .dark class (and therefore the Deck token
            block) applies during onboarding too. */}
        <ThemeProvider>
          <RecordingStateProvider>
            <UpdateStatusProvider>
            <RestartConfirmProvider>
            <CalmMotionProvider>
              <TranscriptProvider>
                <ConfigProvider>
                  <OllamaDownloadProvider>
                    <OnboardingProvider>
                      <SidebarProvider>
                          <TooltipProvider>
                            {/* spec 0051 WS2 — hoisted above RecordingPostProcessingProvider (and
                                out of the showOnboarding ternary, so it's ALWAYS mounted): the
                                tray / global-shortcut stop path runs through
                                RecordingPostProcessingProvider, which calls useRecordingStop
                                unconditionally, and useRecordingStop now reads useBacklog() to
                                hand a 'process-now' meeting to the backlog. That handoff must be
                                live outside onboarding too.
  
                                Being always-mounted means the backlog's mount effect also
                                fires DURING onboarding, when the Rust `AppState` is not yet
                                managed. `api_list_deferred_meetings` therefore uses
                                `try_state` and answers with an empty list on that path
                                (spec 0051 final review, Finding 2) — with `state()` it
                                panicked, and a panicking command never sends its IPC
                                response, so the frontend promise hung forever and the
                                backlog stayed dead for the session. */}
                            <QueueOpenProvider>
                            <DeferredBacklogProvider>
                            <RecordingPostProcessingProvider>
                              <PermissionsModalProvider>
                              <ImportDialogProvider onOpen={handleOpenImportDialog}>
                                {/* specs/0061 W1 Task 2 — AppShell owns the showOnboarding ternary
                                    and mounts DownloadProgressToastProvider (top-right download
                                    toasts) only in the post-onboarding branch: during onboarding,
                                    DownloadProgressStep's own in-page cards already show progress,
                                    so the toast provider used to duplicate that feedback. */}
                                <AppShell showOnboarding={showOnboarding} onOnboardingComplete={handleOnboardingComplete}>
                                  {children}
                                </AppShell>
                                {/* Import audio overlay and dialog */}
                                <ImportDropOverlay visible={showDropOverlay} />
                                <ConditionalImportDialog
                                  showImportDialog={showImportDialog}
                                  handleImportDialogClose={handleImportDialogClose}
                                  importFilePath={importFilePath}
                                />
                              </ImportDialogProvider>
                              </PermissionsModalProvider>
                            </RecordingPostProcessingProvider>
                            </DeferredBacklogProvider>
                            </QueueOpenProvider>
                          </TooltipProvider>
                        </SidebarProvider>
                    </OnboardingProvider>

                  </OllamaDownloadProvider>
                </ConfigProvider>
              </TranscriptProvider>
            </CalmMotionProvider>
            </RestartConfirmProvider>
            </UpdateStatusProvider>
          </RecordingStateProvider>
          <ThemedToaster offset={showOnboarding ? undefined : RAIL_TOAST_OFFSET} />
        </ThemeProvider>
      </body>
    </html>
  )
}
