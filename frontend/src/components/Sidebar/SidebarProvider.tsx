'use client';

import React, { createContext, useContext, useState, useEffect, useRef, useMemo } from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';

/** localStorage key for the persisted sidebar collapse preference ('1' | '0'). */
const SIDEBAR_COLLAPSED_KEY = 'nixon.sidebar.collapsed';


interface SidebarItem {
  id: string;
  title: string;
  type: 'folder' | 'file';
  children?: SidebarItem[];
}

export interface CurrentMeeting {
  id: string;
  title: string;
}

interface SidebarContextType {
  currentMeeting: CurrentMeeting | null;
  setCurrentMeeting: (meeting: CurrentMeeting | null) => void;
  /** The meeting id of the IN-PROGRESS recording (specs/0019 WS6.5). Distinct from
   *  `currentMeeting`, which also tracks the meeting the user is merely *viewing* — so it
   *  gets overwritten when you open another meeting mid-recording. This is set only at
   *  recording start and cleared at stop, so "is this the live recording?" checks (e.g. the
   *  meeting-details → /record redirect) stay correct regardless of navigation. */
  activeRecordingMeetingId: string | null;
  setActiveRecordingMeetingId: (id: string | null) => void;
  sidebarItems: SidebarItem[];
  isCollapsed: boolean;
  toggleCollapse: () => void;
  meetings: CurrentMeeting[];
  setMeetings: (meetings: CurrentMeeting[]) => void;
  isMeetingActive: boolean;
  setIsMeetingActive: (active: boolean) => void;
  handleRecordingToggle: () => void;
  handleNewNote: () => Promise<void>;
  // Summary polling management
  startSummaryPolling: (meetingId: string, processId: string, onUpdate: (result: any) => void) => void;
  stopSummaryPolling: (meetingId: string) => void;
  // Refetch meetings from backend
  refetchMeetings: () => Promise<void>;

}

const SidebarContext = createContext<SidebarContextType | null>(null);

export const useSidebar = () => {
  const context = useContext(SidebarContext);
  if (!context) {
    throw new Error('useSidebar must be used within a SidebarProvider');
  }
  return context;
};

export function SidebarProvider({ children }: { children: React.ReactNode }) {
  const [currentMeeting, setCurrentMeeting] = useState<CurrentMeeting | null>({ id: 'intro-call', title: '+ New Call' });
  // specs/0019 WS6.5 — the live recording's meeting id, immune to view-navigation.
  const [activeRecordingMeetingId, setActiveRecordingMeetingId] = useState<string | null>(null);
  // The collapse state is PERSISTED (localStorage). It used to be plain in-memory
  // state defaulting to collapsed, so any remount of this provider — which happens
  // once shortly after first load as the app settles into its steady-state tree —
  // snapped the sidebar shut on the first navigation even after the user opened it.
  // Persisting + re-reading on mount makes it immune to remounts and remembers the
  // user's preference across launches.
  const [isCollapsed, setIsCollapsed] = useState(true);
  const [meetings, setMeetings] = useState<CurrentMeeting[]>([]);
  const [sidebarItems, setSidebarItems] = useState<SidebarItem[]>([]);
  const [isMeetingActive, setIsMeetingActive] = useState(false);
  // Summary-poll intervals live in a ref (not state) so the mount-only unmount cleanup below
  // closes over a *stable* container. Previously these were held in state and the cleanup
  // effect depended on that state, so every add/remove re-ran the effect and its teardown
  // cleared ALL concurrent polls — one meeting's poll starting/stopping killed the others
  // (spec 0028, High). A ref decouples "which polls exist" from the component lifecycle.
  const activeSummaryPollsRef = useRef<Map<string, NodeJS.Timeout>>(new Map());

  // Use recording state from RecordingStateContext (single source of truth)
  const { isRecording } = useRecordingState();

  const pathname = usePathname();
  const router = useRouter();

  // Extract fetchMeetings as a reusable function
  const fetchMeetings = React.useCallback(async () => {
    try {
      const meetings = await invoke<Array<{ id: string; title: string }>>('api_get_meetings');
      const transformedMeetings = meetings.map((meeting) => ({
        id: meeting.id,
        title: meeting.title
      }));
      setMeetings(transformedMeetings);
    } catch (error) {
      console.error('Error fetching meetings:', error);
      setMeetings([]);
    }
  }, []);

  useEffect(() => {
    fetchMeetings();
  }, [fetchMeetings]);

  const baseItems: SidebarItem[] = [
    {
      id: 'meetings',
      title: 'Meeting Notes',
      type: 'folder' as const,
      children: [
        ...meetings.map(meeting => ({ id: meeting.id, title: meeting.title, type: 'file' as const }))
      ]
    },
  ];


  // Restore the persisted preference on mount. Running in an effect (not a lazy
  // useState initializer) keeps SSR/hydration clean, and — crucially — re-applies
  // the stored value after any provider remount, so the sidebar no longer resets to
  // collapsed on the first navigation.
  useEffect(() => {
    try {
      const stored = window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY);
      if (stored !== null) setIsCollapsed(stored === '1');
    } catch {
      /* localStorage unavailable — fall back to the collapsed default. */
    }
  }, []);

  const toggleCollapse = () => {
    setIsCollapsed((prev) => {
      const next = !prev;
      try {
        window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, next ? '1' : '0');
      } catch {
        /* ignore — preference just won't persist this session. */
      }
      return next;
    });
  };

  // Update current meeting when on the recorder page.
  //
  // `intro-call` is a no-meeting sentinel ("+ New Call"): it marks "we're on the recorder but
  // haven't started a recording yet". Persist-at-start (spec 0007) creates the REAL meeting row
  // when recording begins and sets currentMeeting to its id, so we must NOT overwrite that here
  // while a recording is active (this effect can re-fire on remount/HMR). Only reset to the
  // placeholder when idle.
  useEffect(() => {
    if (pathname === '/record' && !isRecording) {
      setCurrentMeeting({ id: 'intro-call', title: '+ New Call' });
    }
    setSidebarItems(baseItems);
  }, [pathname, isRecording]);

  // Update sidebar items when meetings change
  useEffect(() => {
    setSidebarItems(baseItems);
  }, [meetings]);

  // Function to handle recording toggle from sidebar
  const handleRecordingToggle = () => {
    if (!isRecording) {
      // Check if already on the recorder page
      if (pathname === '/record') {
        // Already on the recorder - trigger recording directly via custom event
        console.log('Triggering recording from sidebar (already on recorder page)');
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      } else {
        // Not on recorder - navigate and use auto-start mechanism
        console.log('Navigating to recorder page with auto-start flag');
        sessionStorage.setItem('autoStartRecording', 'true');
        router.push('/record');
      }
    }
    // The actual recording start/stop is handled in the Home component
  };

  // Create a notes-only meeting (spec 0015 Phase B): a real `meetings` row with
  // origin='notes_only' and NO recording. Mirrors `createMeetingForRecording` but
  // starts no audio — it just opens the notes-only meeting-details view. The notepad
  // there autosaves to `meeting_notes`; a notes-grounded summary still applies.
  const handleNewNote = React.useCallback(async () => {
    try {
      const result = await invoke<{ meeting_id: string }>('api_create_meeting', {
        meetingTitle: 'Untitled note',
        origin: 'notes_only',
      });
      const newId = result?.meeting_id;
      if (!newId) {
        throw new Error('api_create_meeting returned no meeting_id');
      }
      setCurrentMeeting({ id: newId, title: 'Untitled note' });
      await fetchMeetings();
      router.push(`/meeting-details?id=${newId}`);
    } catch (error) {
      console.error('Failed to create notes-only meeting:', error);
      toast.error('Could not create a new note', {
        description: 'Please try again.',
      });
    }
  }, [fetchMeetings, router]);

  // Summary polling management
  const startSummaryPolling = React.useCallback((
    meetingId: string,
    processId: string,
    onUpdate: (result: any) => void
  ) => {
    // Stop existing poll for this meeting if any
    if (activeSummaryPollsRef.current.has(meetingId)) {
      clearInterval(activeSummaryPollsRef.current.get(meetingId)!);
    }

    console.log(`📊 Starting polling for meeting ${meetingId}, process ${processId}`);

    let pollCount = 0;
    const MAX_POLLS = 200; // ~16.5 minutes at 5-second intervals (slightly longer than backend's 15-min timeout to avoid race conditions)

    const pollInterval = setInterval(async () => {
      pollCount++;

      // Timeout safety: Stop after 10 minutes
      if (pollCount >= MAX_POLLS) {
        console.warn(`⏱️ Polling timeout for ${meetingId} after ${MAX_POLLS} iterations`);
        clearInterval(pollInterval);
        activeSummaryPollsRef.current.delete(meetingId);
        onUpdate({
          status: 'error',
          error: 'Summary generation timed out after 15 minutes. Please try again or check your model configuration.'
        });
        return;
      }
      try {
        const result = await invoke('api_get_summary', {
          meetingId: meetingId,
        }) as any;

        console.log(`📊 Polling update for ${meetingId}:`, result.status);

        // Call the update callback with result
        onUpdate(result);

        // Stop polling if completed, error, failed, cancelled, or idle (after initial processing)
        if (result.status === 'completed' || result.status === 'error' || result.status === 'failed' || result.status === 'cancelled') {
          console.log(`Polling completed for ${meetingId}, status: ${result.status}`);
          clearInterval(pollInterval);
          activeSummaryPollsRef.current.delete(meetingId);
        } else if (result.status === 'idle' && pollCount > 1) {
          // If we get 'idle' after polling started, process completed/disappeared
          console.log(`Process completed or not found for ${meetingId}, stopping poll`);
          clearInterval(pollInterval);
          activeSummaryPollsRef.current.delete(meetingId);
        }
      } catch (error) {
        console.error(`Polling error for ${meetingId}:`, error);
        // Report error to callback
        onUpdate({
          status: 'error',
          error: error instanceof Error ? error.message : 'Unknown error'
        });
        clearInterval(pollInterval);
        activeSummaryPollsRef.current.delete(meetingId);
      }
    }, 5000); // Poll every 5 seconds

    activeSummaryPollsRef.current.set(meetingId, pollInterval);
  }, []);

  const stopSummaryPolling = React.useCallback((meetingId: string) => {
    const pollInterval = activeSummaryPollsRef.current.get(meetingId);
    if (pollInterval) {
      console.log(`⏹️ Stopping polling for meeting ${meetingId}`);
      clearInterval(pollInterval);
      activeSummaryPollsRef.current.delete(meetingId);
    }
  }, []);

  // Cleanup all polling intervals on unmount ONLY. Depending on the interval map here (as the
  // old code did) made this teardown fire on every poll add/remove and clear every concurrent
  // poll; a mount-only effect over the stable ref clears intervals exactly once, at unmount.
  useEffect(() => {
    const polls = activeSummaryPollsRef.current;
    return () => {
      console.log('🧹 Cleaning up all summary polling intervals');
      polls.forEach(interval => clearInterval(interval));
      polls.clear();
    };
  }, []);



  // Memoize the context value so consumers don't re-render on every SidebarProvider render
  // (spec 0028, Medium — mirrors RecordingStateContext). The inline-object literal used
  // before produced a new reference each render, storming every useSidebar() consumer.
  const contextValue = useMemo<SidebarContextType>(() => ({
    currentMeeting,
    setCurrentMeeting,
    activeRecordingMeetingId,
    setActiveRecordingMeetingId,
    sidebarItems,
    isCollapsed,
    toggleCollapse,
    meetings,
    setMeetings,
    isMeetingActive,
    setIsMeetingActive,
    handleRecordingToggle,
    handleNewNote,
    startSummaryPolling,
    stopSummaryPolling,
    refetchMeetings: fetchMeetings,
  }), [
    currentMeeting,
    activeRecordingMeetingId,
    sidebarItems,
    isCollapsed,
    meetings,
    isMeetingActive,
    handleNewNote,
    startSummaryPolling,
    stopSummaryPolling,
    fetchMeetings,
    // Closure deps of the inline handler above (handleRecordingToggle)
    // so the memoized value never captures a stale copy.
    isRecording,
    pathname,
    router,
  ]);

  return (
    <SidebarContext.Provider value={contextValue}>
      {children}
    </SidebarContext.Provider>
  );
}
