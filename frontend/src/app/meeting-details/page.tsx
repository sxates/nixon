"use client"
import { useSidebar } from "@/components/Sidebar/SidebarProvider";
import { useState, useEffect, useCallback, useRef, Suspense } from "react";
import { Transcript, Summary } from "@/types";
import PageContent from "./page-content";
import { useRouter, useSearchParams } from "next/navigation";
import { invoke } from "@tauri-apps/api/core";
import { LoaderIcon } from "lucide-react";
import { toast } from "sonner";
import { useConfig } from "@/contexts/ConfigContext";
import { useRecordingState } from "@/contexts/RecordingStateContext";
import { useBacklog } from "@/contexts/DeferredBacklogProvider";
import { isMeetingInFlight } from "@/lib/deferred-backlog";
import { usePaginatedTranscripts } from "@/hooks/usePaginatedTranscripts";
import { useSegmentDeepLink } from "@/hooks/useSegmentDeepLink";
import { shouldRedirectToActiveRecording } from "@/lib/recording-redirect";
import {
  AutoSummarySkipReason,
  autoSummarySkipReason,
  autoSummarySkipToast,
} from "@/lib/auto-summary";
import { devLog } from "@/lib/dev-log";

interface MeetingDetailsResponse {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  transcripts: Transcript[];
  folder_path?: string;
  // Meeting type (spec 0015 + 0036 'scheduled'). Absent ⇒ 'recorded'. Drives the notes-only /
  // prep views.
  origin?: 'recorded' | 'notes_only' | 'imported' | 'scheduled';
  // Calendar linkage (specs/0036) — present on scheduled/calendar-linked rows. Threaded so
  // the scheduled-meeting record banner can bind a recording to this event (keeps the roster).
  calendarEventId?: string | null;
  calendarSeriesKey?: string | null;
  // Archival reel ordinal (specs/0057) — the identity line's "REEL 0412" handle.
  reelNumber?: number;
  // specs/0069b review fix 2 — manual-entry occurrence end + join link (edit dialog
  // seed) and whether this row is a manual entry at all. Backend-computed; see
  // MeetingMetadata in @/types for the prefix-check rationale.
  scheduledEndAt?: string | null;
  joinUrl?: string | null;
  isManualEntry?: boolean;
}

function MeetingDetailsContent() {
  const searchParams = useSearchParams();
  const meetingId = searchParams.get('id');
  const source = searchParams.get('source'); // Check if navigated from recording
  // specs/0033 — search deep-link: scroll the transcript to this segment. Best-effort
  // (segment ids regenerate on re-transcription), so a stale id silently no-ops. The
  // param only SEEDS the intent — useSegmentDeepLink below owns its lifecycle.
  const segmentParam = searchParams.get('segment');
  const { setCurrentMeeting, refetchMeetings, stopSummaryPolling, isMeetingActive, activeRecordingMeetingId } = useSidebar();
  const { isAutoSummary } = useConfig(); // Get auto-summary toggle state
  const { isRecording } = useRecordingState();
  // spec 0051 final review (Finding 1): the deferred backlog is the only thing that knows
  // whether this meeting's 'defer' marker means "waiting" or "being processed right now"
  // (a successful 'process-now' stop marks 'defer' and hands off; the marker is cleared
  // only at the END of the pipeline). Read through the same shared provider
  // TranscriptButtonGroup uses so the toast and the backlog pill can never contradict.
  const { view: backlogView } = useBacklog();
  /** The meeting-summary fetch, so the backlog watcher can re-run it (see below). */
  const fetchSummaryRef = useRef<null | (() => Promise<void>)>(null);
  const backlogItems = backlogView.items;
  const router = useRouter();

  // If the meeting being opened is the one currently being recorded, send the user to the
  // live recording interface instead of this (empty, not-yet-saved) summary view. The
  // in-progress meeting is a real SQLite row created at recording start (persist-at-start),
  // so the sidebar links it to /meeting-details — but it has no summary yet and its live
  // transcript/notes only exist on /record.
  //
  // specs/0019 WS6.5: match against `activeRecordingMeetingId` (set only at recording
  // start/stop), NOT `currentMeeting.id` — the latter is overwritten to whatever meeting
  // you VIEW, so opening a past meeting mid-recording used to match here and wrongly
  // redirect you to /record. The redirect runs before the loading/blank-summary UI below.
  const isLiveRecordingMeeting = shouldRedirectToActiveRecording({
    isRecording,
    isMeetingActive,
    meetingId,
    activeRecordingMeetingId,
  });

  useEffect(() => {
    if (isLiveRecordingMeeting) {
      console.log('[MeetingDetails] Meeting is currently recording — redirecting to /record');
      router.replace('/record');
    }
  }, [isLiveRecordingMeeting, router]);
  const [meetingDetails, setMeetingDetails] = useState<MeetingDetailsResponse | null>(null);
  const [meetingSummary, setMeetingSummary] = useState<Summary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [shouldAutoGenerate, setShouldAutoGenerate] = useState<boolean>(false);
  const [hasCheckedAutoGen, setHasCheckedAutoGen] = useState<boolean>(false);

  // Use pagination hook for efficient transcript loading
  const {
    metadata,
    segments,
    transcripts,
    isLoading: isTranscriptsLoading,
    isLoadingMore,
    hasMore,
    totalCount,
    loadedCount,
    loadMore,
    refetch,
    error: transcriptError,
  } = usePaginatedTranscripts({ meetingId: meetingId || '' });

  // specs/0033 — consume-then-clear: once the deep-link scroll succeeds (or is
  // abandoned as stale), strip `segment` from the URL while keeping `id` & co, so
  // re-selecting the same search hit is a real param transition that re-fires the
  // chain instead of a silent no-op (identical URL → unchanged searchParams).
  const clearSegmentParam = useCallback(() => {
    const params = new URLSearchParams(searchParams.toString());
    if (!params.has('segment')) return;
    params.delete('segment');
    const qs = params.toString();
    router.replace(qs ? `/meeting-details?${qs}` : '/meeting-details', { scroll: false });
  }, [searchParams, router]);

  // Deep-link lifecycle: holds the pending scroll target in state (so clearing the
  // URL param doesn't cancel an in-flight scroll), actively pages the transcript
  // until the target segment is loaded, and abandons stale ids (silent no-op).
  const {
    pendingSegmentId,
    consume: consumeSegmentDeepLink,
    // specs/0061 W4 (task 3) — code-driven scroll intent (a clicked speaker's
    // first line), exposed to PageContent's onSelectSpeaker.
    request: requestSegmentScroll,
  } = useSegmentDeepLink({
    meetingId,
    segmentParam,
    segments,
    hasMore,
    isLoading: isTranscriptsLoading,
    isLoadingMore,
    loadMore,
    onClearParam: clearSegmentParam,
  });

  // specs/0029 WS7.3 — when we land here fresh off a recording (?source=recording) and
  // auto-generation is skipped, tell the user why instead of a silent console.log. One
  // subtle toast per visit; normal navigation (source !== 'recording') never toasts.
  const skipToastShownRef = useRef(false);
  const reportAutoGenSkip = useCallback((reason: AutoSummarySkipReason) => {
    console.log('[AutoSummary] Skipped:', reason);
    const copy = autoSummarySkipToast(reason);
    if (copy && !skipToastShownRef.current) {
      skipToastShownRef.current = true;
      toast.info(copy.title, { description: copy.description });
    }
  }, []);

  // Check if gemma3:1b model is available in Ollama
  const checkForGemmaModel = useCallback(async (): Promise<boolean> => {
    try {
      const models = await invoke('get_ollama_models', { endpoint: null }) as any[];
      const hasGemma = models.some((m: any) => m.name === 'gemma3:1b');
      console.log('🔍 Checked for gemma3:1b:', hasGemma);
      return hasGemma;
    } catch (error) {
      console.error('❌ Failed to check Ollama models:', error);
      return false;
    }
  }, []);

  // Set up auto-generation - respects DB as source of truth
  const setupAutoGeneration = useCallback(async (transcriptCount: number) => {
    if (hasCheckedAutoGen) return; // Only check once

    // specs/0029 WS7.2: a record-only meeting has zero transcripts but recorded
    // audio awaiting deferred transcription — auto-summary must proceed (the summary
    // flow transcribes first) instead of skipping with "transcript is empty".
    // Probed only for the empty case; failures leave the classic gate untouched.
    let audioAwaitingTranscription = false;
    if (transcriptCount <= 0 && source === 'recording') {
      try {
        audioAwaitingTranscription = await invoke<boolean>('api_meeting_audio_available', {
          meetingId,
        });
      } catch (error) {
        console.warn('Could not check audio availability for auto-summary gate:', error);
      }
    }

    // 1.10 feedback: a deferred meeting (or one whose immediate go-live pass is
    // running) is owned by the deferred-processing machinery — auto-summary must
    // not fire here. Probe failures leave the marker unknown (null → no gate).
    let processingMode: string | null = null;
    if (source === 'recording') {
      try {
        processingMode = await invoke<string | null>('api_get_meeting_processing_mode', {
          meetingId,
        });
      } catch (error) {
        console.warn('Could not check processing mode for auto-summary gate:', error);
      }
    }

    // Synchronous gates (source / deferral / toggle / transcript) — pure and testable
    // (specs/0029 WS7.3). Model availability is probed below, so pass it as true here.
    const preReason = autoSummarySkipReason({
      source,
      isAutoSummaryEnabled: isAutoSummary,
      transcriptCount,
      hasModelConfigured: true,
      audioAwaitingTranscription,
      processingMode,
      isProcessingInBacklog: isMeetingInFlight(backlogItems, meetingId),
    });
    if (preReason) {
      if (preReason === 'not-from-recording') {
        console.log('Not from recording navigation, skipping auto-generation');
      } else {
        console.log('Auto-summary skipped:', preReason);
      }
      reportAutoGenSkip(preReason);
      setHasCheckedAutoGen(true);
      return;
    }

    try {
      // Check what's currently in database
      const currentConfig = await invoke('api_get_model_config') as any;

      // If DB already has a model, use it (never override!)
      if (currentConfig && currentConfig.model) {
        console.log('Using existing model from DB:', currentConfig.model);
        setShouldAutoGenerate(true);
        setHasCheckedAutoGen(true);
        return;
      }

      // DB is empty - check if gemma3:1b exists as fallback
      const hasGemma = await checkForGemmaModel();

      if (hasGemma) {
        console.log('💾 DB empty, using gemma3:1b as initial default');

        await invoke('api_save_model_config', {
          provider: 'ollama',
          model: '',
          whisperModel: 'large-v3',
          apiKey: null,
          ollamaEndpoint: null,
        });

        setShouldAutoGenerate(true);
      } else {
        console.log('⚠️ No model configured and gemma3:1b not found');
        reportAutoGenSkip('no-model');
      }
    } catch (error) {
      console.error('❌ Failed to setup auto-generation:', error);
      reportAutoGenSkip('model-check-failed');
    }

    setHasCheckedAutoGen(true);
  }, [hasCheckedAutoGen, checkForGemmaModel, source, isAutoSummary, reportAutoGenSkip, meetingId, backlogItems]);

  // Sync meeting metadata from pagination hook to meeting details state
  useEffect(() => {
    if (metadata && (!meetingId || meetingId === 'intro-call')) {
      // If invalid meeting ID, don't sync
      return;
    }

    if (metadata) {
      console.log('Meeting metadata loaded:', metadata);

      // Build meeting details from metadata and paginated transcripts
      setMeetingDetails({
        id: metadata.id,
        title: metadata.title,
        created_at: metadata.created_at,
        updated_at: metadata.updated_at,
        transcripts: transcripts, // Paginated transcripts from hook
        folder_path: metadata.folder_path, // For retranscription feature
        origin: metadata.origin, // Meeting type (spec 0015) — drives notes-only view
        calendarEventId: metadata.calendarEventId, // scheduled-meeting record binding (specs/0036)
        calendarSeriesKey: metadata.calendarSeriesKey,
        reelNumber: metadata.reelNumber, // specs/0057 — reel label + identity line
        scheduledEndAt: metadata.scheduledEndAt, // specs/0069b review fix 2 — edit dialog seed
        joinUrl: metadata.joinUrl,
        isManualEntry: metadata.isManualEntry,
      });

      // Sync with sidebar context
      setCurrentMeeting({ id: metadata.id, title: metadata.title });
    }
  }, [metadata, transcripts, meetingId, setCurrentMeeting]);

  // Handle transcript loading errors
  useEffect(() => {
    if (transcriptError) {
      console.error('Error loading transcripts:', transcriptError);
      setError(transcriptError);
    }
  }, [transcriptError]);

  // Extract fetchMeetingDetails for use in child components (now refetches via hook)
  const fetchMeetingDetails = useCallback(async () => {
    if (!meetingId || meetingId === 'intro-call') {
      return;
    }

    // The usePaginatedTranscripts hook automatically refetches when meetingId changes
    // This function is kept for compatibility with onMeetingUpdated callback
    console.log('fetchMeetingDetails called - pagination hook will handle refetch');
  }, [meetingId]);

  // Reset states when meetingId changes (prevent race conditions)
  useEffect(() => {
    setMeetingDetails(null);
    setMeetingSummary(null);
    setError(null);
    setIsLoading(true);
    // Reset auto-generation state to allow new meeting to be checked
    setHasCheckedAutoGen(false);
    setShouldAutoGenerate(false);
    skipToastShownRef.current = false; // allow one skip toast per meeting visit
  }, [meetingId]);

  // Cleanup: Stop polling when navigating away from a meeting
  useEffect(() => {
    return () => {
      if (meetingId) {
        console.log('Cleaning up: Stopping summary polling for meeting:', meetingId);
        stopSummaryPolling(meetingId);
      }
    };
  }, [meetingId, stopSummaryPolling]);

  useEffect(() => {
    console.log('MeetingDetails useEffect triggered - meetingId:', meetingId);

    if (!meetingId || meetingId === 'intro-call') {
      console.warn('No valid meeting ID in URL - meetingId:', meetingId);
      setError("No meeting selected");
      setIsLoading(false);
      return;
    }

    console.log('Valid meeting ID found, fetching details for:', meetingId);

    setMeetingDetails(null);
    setMeetingSummary(null);
    setError(null);
    setIsLoading(true);

    const fetchMeetingSummary = async () => {
      try {
        const summary = await invoke('api_get_summary', {
          meetingId: meetingId,
        }) as any;

        devLog('FETCH SUMMARY: Raw response:', summary);

        // Check if the summary request failed with 404 or error status, or if no summary exists yet (idle)
        // Note: 'cancelled' and 'failed' statuses can still have data if backup was restored
        if (summary.status === 'idle' || (!summary.data && summary.status === 'error')) {
          console.warn('Meeting summary not found or no summary generated yet:', summary.error || 'idle');
          setMeetingSummary(null);
          return;
        }

        const summaryData = summary.data || {};

        // Parse if it's a JSON string (backend may return double-encoded JSON)
        let parsedData = summaryData;
        if (typeof summaryData === 'string') {
          try {
            parsedData = JSON.parse(summaryData);
          } catch (_e) {
            parsedData = {};
          }
        }

        console.log('🔍 FETCH SUMMARY: Parsed data:', parsedData);

        // Priority 1: BlockNote JSON format
        if (parsedData.summary_json) {
          setMeetingSummary(parsedData as any);
          return;
        }

        // Priority 2: Markdown format
        if (parsedData.markdown) {
          setMeetingSummary(parsedData as any);
          return;
        }

        // Legacy format - apply formatting
        console.log('LEGACY FORMAT: Detected legacy format, applying section formatting');

        const { MeetingName: _MeetingName, _section_order, ...restSummaryData } = parsedData;

        // Format the summary data with consistent styling - PRESERVE ORDER
        const formattedSummary: Summary = {};

        // Use section order if available to maintain exact order and handle duplicates
        const sectionKeys = _section_order || Object.keys(restSummaryData);

        console.log('LEGACY FORMAT: Processing sections:', sectionKeys);

        for (const key of sectionKeys) {
          try {
            const section = restSummaryData[key];
            // Comprehensive null checks to prevent the error
            if (section &&
              typeof section === 'object' &&
              'title' in section &&
              'blocks' in section) {
              const typedSection = section as { title?: string; blocks?: any[] };

              // Ensure blocks is an array before mapping
              if (Array.isArray(typedSection.blocks)) {
                formattedSummary[key] = {
                  title: typedSection.title || key,
                  blocks: typedSection.blocks.map((block: any) => ({
                    ...block,
                    // type: 'bullet',
                    color: 'default',
                    content: block?.content?.trim() || ''
                  }))
                };
              } else {
                // Handle case where blocks is not an array
                console.warn(`LEGACY FORMAT: Section ${key} has invalid blocks:`, typedSection.blocks);
                formattedSummary[key] = {
                  title: typedSection.title || key,
                  blocks: []
                };
              }
            } else {
              console.warn(`LEGACY FORMAT: Skipping invalid section ${key}:`, section);
            }
          } catch (error) {
            console.warn(`LEGACY FORMAT: Error processing section ${key}:`, error);
            // Continue processing other sections
          }
        }

        devLog('LEGACY FORMAT: Formatted summary:', formattedSummary);
        setMeetingSummary(formattedSummary);
      } catch (error) {
        console.error('FETCH SUMMARY: Error fetching meeting summary:', error);
        // Don't set error state for summary fetch failure, set to null to show generate button
        setMeetingSummary(null);
      }
    };

    // Held so the backlog watcher below can re-run exactly this fetch — same command, same
    // parsing, no duplication and no loading flash.
    fetchSummaryRef.current = fetchMeetingSummary;

    const loadData = async () => {
      try {
        await fetchMeetingSummary();
      } finally {
        setIsLoading(false);
      }
    };

    loadData();
  }, [meetingId]);

  // The summary is fetched once, keyed on the meeting id — so a summary the DEFERRED
  // BACKLOG produces afterwards never reached the page. Measured 2026-09-22: the drain
  // logged `summary -> done` and the row was stored `completed` with 2137 characters, while
  // the open meeting showed nothing; the specs/0071 W3 "Summarizing this meeting…" notice
  // simply vanished and left an empty tab behind. The page polls only for summaries IT
  // started (`useSummaryGeneration`), and this one was started by the backlog.
  //
  // So: when this meeting leaves the backlog's in-flight set, re-read the summary.
  const wasInFlightRef = useRef(false);
  useEffect(() => {
    const inFlight = isMeetingInFlight(backlogView.items, meetingId);
    if (wasInFlightRef.current && !inFlight) {
      void fetchSummaryRef.current?.();
    }
    wasInFlightRef.current = inFlight;
  }, [backlogView.items, meetingId]);

  // Auto-generation check: runs when meeting is loaded with no summary
  useEffect(() => {
    const checkAutoGen = async () => {
      // Only auto-generate if:
      // 1. We have meeting details
      // 2. No summary exists
      // 3. Haven't checked yet
      if (!meetingDetails || meetingSummary !== null || hasCheckedAutoGen) {
        return;
      }

      // specs/0029 WS7.3 — transcripts load async (metadata lands first, the first
      // transcript page a beat later). Don't decide "transcript is empty" until the
      // initial load settles; a non-empty count can proceed immediately.
      const transcriptCount = meetingDetails.transcripts?.length ?? 0;
      if (transcriptCount === 0 && isTranscriptsLoading) {
        return; // effect re-runs when loading finishes / transcripts arrive
      }

      console.log('No summary found, checking for auto-generation...');
      await setupAutoGeneration(transcriptCount);
    };

    checkAutoGen();
  }, [meetingDetails, meetingSummary, hasCheckedAutoGen, setupAutoGeneration, isTranscriptsLoading]);

  // While redirecting an in-progress recording to /record, show a spinner instead of the
  // blank summary so the user never sees the empty pane flash.
  if (isLiveRecordingMeeting) {
    return (
      <div className="flex items-center justify-center h-page">
        <LoaderIcon className="animate-spin size-6" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-page">
        <div className="text-center">
          <p className="text-destructive mb-4">{error}</p>
          <button
            onClick={() => router.push('/')}
            className="px-4 py-2 bg-brand text-brand-foreground rounded hover:bg-brand/90"
          >
            Go Back
          </button>
        </div>
      </div>
    );
  }

  // Show the spinner only on the INITIAL load (no meeting details yet). A later
  // refetch (e.g. after renaming a speaker) keeps the populated meetingDetails, so
  // PageContent stays mounted and the active tab / speaker legend aren't reset.
  if (isLoading || !meetingDetails) {
    return <div className="flex items-center justify-center h-page">
      <LoaderIcon className="animate-spin size-6 " />
    </div>;
  }

  return <PageContent
    meeting={meetingDetails}
    summaryData={meetingSummary}
    shouldAutoGenerate={shouldAutoGenerate}
    onAutoGenerateComplete={() => setShouldAutoGenerate(false)}
    isProcessingInBacklog={isMeetingInFlight(backlogItems, meetingId)}
    onMeetingUpdated={async () => {
      // Refetch meeting details to get updated title from backend
      await fetchMeetingDetails();
      // Refetch meetings list to update sidebar
      await refetchMeetings();
    }}
    onRefetchTranscripts={refetch}
    // specs/0033 — search deep-link target segment (transcript hits). The child
    // reports back via onDeepLinkConsumed once the scroll ran (or can't ever run),
    // which clears the pending state AND the `?segment=` URL param.
    deepLinkSegmentId={pendingSegmentId}
    onDeepLinkConsumed={consumeSegmentDeepLink}
    requestSegmentScroll={requestSegmentScroll}
    // Pagination props for efficient transcript loading
    segments={segments}
    hasMore={hasMore}
    isLoadingMore={isLoadingMore}
    totalCount={totalCount}
    loadedCount={loadedCount}
    onLoadMore={loadMore}
  />;
}

export default function MeetingDetails() {
  return (
    <Suspense fallback={
      <div className="flex items-center justify-center h-page">
        <LoaderIcon className="animate-spin size-6" />
      </div>
    }>
      <MeetingDetailsContent />
    </Suspense>
  );
}
