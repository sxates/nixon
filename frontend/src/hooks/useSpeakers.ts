/**
 * Speaker legend + rename/merge/attendee-association controller (specs/0010, P2, Task 8).
 *
 * Builds on the P1 diarization frontend: once a meeting has been diarized,
 * `api_get_meeting_speakers` returns its speakers. This hook owns the post-diarization
 * editing surface:
 *   - fetch the meeting's speakers (and the linked calendar event's attendees, if any)
 *   - rename a speaker            → `api_rename_speaker`
 *   - name a speaker from an
 *     attendee (display + email)  → `api_assign_speaker_to_attendee`
 *   - merge two speakers          → `api_merge_speakers` (destructive of `fromKey`)
 *
 * After any mutation we re-fetch the speakers AND ask the caller to re-fetch the
 * transcript, because the display names shown per segment come from the backend
 * join (transcripts → speakers); local state alone wouldn't update the labels.
 *
 * Attendees: the getter may return an empty roster (no linked calendar event) — that
 * is the common case and is NOT an error; callers fall back to free-text rename.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import { notifyMeetingParticipantsChanged } from '@/lib/participants-events';
import {
  useAudioSetup,
  type AudioSetupOverride,
  type AudioSetupStartResult,
  type DiarizationCompleteSetupPayload,
  type MeetingAudioSetup,
} from '@/hooks/useAudioSetup';
import { buildSpeakerPhotoMap, type SpeakerPhotoMap } from '@/lib/speaker-photos';
import type {
  MeetingSpeaker,
  MeetingAttendee,
  AttendeeSuggestion,
  MeetingAttendeesResponse,
  SpeakerSuggestion,
  Person,
} from '@/types';

/**
 * Cross-meeting suggestion dismissals are sticky *per meeting* but transient
 * across app restarts — we use sessionStorage (not localStorage) so a dismissed
 * "Looks like Priya" stays gone for this viewing session without permanently
 * suppressing a suggestion the user may want to revisit later. Keyed by
 * meetingId + speakerKey, mirroring the calendar-connect prompt's dismissal
 * pattern (which uses localStorage for a longer-lived "don't ask again").
 */
const SUGGESTION_DISMISS_PREFIX = 'nixon-speaker-suggestion-dismissed';
const suggestionDismissKey = (meetingId: string, speakerKey: string) =>
  `${SUGGESTION_DISMISS_PREFIX}:${meetingId}:${speakerKey}`;

interface UseSpeakersOptions {
  meetingId: string | undefined;
  /** Called after a mutation so the caller can re-fetch the transcript (labels
   *  come from the backend join, so the transcript view must reload too). */
  onMutated?: () => void | Promise<void>;
}

export interface UseSpeakersReturn {
  speakers: MeetingSpeaker[];
  attendees: MeetingAttendee[];
  /** Durable People (specs/0016 1b) offered as an additional assign source in the
   *  pick-list, alongside calendar attendees. Best-effort; empty on failure. */
  people: Person[];
  /** Pre-computed obvious 1:1 mapping (may be null), surfaced as a one-click action. */
  suggestion: AttendeeSuggestion | null;
  /** Cross-meeting voice-identity suggestions (specs/0016 1a), keyed by speakerKey.
   *  Already filtered to exclude ones the user dismissed for this meeting. */
  crossMeetingSuggestions: Map<string, SpeakerSuggestion>;
  /** Dismiss a cross-meeting suggestion for a speaker; it won't be re-offered this
   *  meeting (sticky via sessionStorage, keyed by meetingId+speakerKey). */
  dismissSuggestion: (speakerKey: string) => void;
  isLoading: boolean;
  /** Re-fetch speakers + attendees (e.g. after diarization completes). */
  refresh: () => Promise<void>;
  renameSpeaker: (speakerKey: string, displayName: string) => Promise<void>;
  assignAttendee: (
    speakerKey: string,
    attendee: { name: string; email: string },
  ) => Promise<void>;
  /** Link a detected speaker to a durable Person (specs/0016 1b). Works regardless
   *  of the person's voiceprint opt-out. */
  assignPerson: (speakerKey: string, person: Person) => Promise<void>;
  mergeSpeakers: (fromKey: string, intoKey: string) => Promise<void>;
  /** specs/0078 — the meeting's "Who was on the mic?" setup; `resolved` decides
   *  whether "This is me" / "This isn't me" are offered. null until fetched. */
  audioSetup: MeetingAudioSetup | null;
  /** Store a "Who was on the mic?" override and start the re-run (the "…" submenu).
   *  Updates `audioSetup` above, so the owner actions and the submenu never disagree. */
  setAudioSetup: (setup: AudioSetupOverride) => Promise<AudioSetupStartResult>;
  /** Re-read the audio setup from the backend. */
  refetchAudioSetup: () => Promise<void>;
  /** specs/0078 "This is me": re-key these speakers to "You". Several keys = one
   *  consolidated group; the backend folds each into the same "You". */
  markAsMe: (speakerKeys: string | string[]) => Promise<void>;
  /** specs/0078 "This isn't me": this meeting's "You" becomes the next "Speaker N". */
  unmarkMe: () => Promise<void>;
  /** speakerKey → cached directory photo, resolved once from speakers + attendees + people
   *  (`buildSpeakerPhotoMap`). Read by the transcript's run headers and the legend. */
  speakerPhotos: SpeakerPhotoMap;
}

export function useSpeakers({
  meetingId,
  onMutated,
}: UseSpeakersOptions): UseSpeakersReturn {
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [attendees, setAttendees] = useState<MeetingAttendee[]>([]);
  const [people, setPeople] = useState<Person[]>([]);
  const [suggestion, setSuggestion] = useState<AttendeeSuggestion | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  // Raw cross-meeting suggestions (specs/0016 1a) keyed by speakerKey, plus the
  // set of speakerKeys the user has dismissed for this meeting. The exposed map
  // is the difference of the two.
  const [rawSuggestions, setRawSuggestions] = useState<
    Map<string, SpeakerSuggestion>
  >(new Map());
  const [dismissedKeys, setDismissedKeys] = useState<Set<string>>(new Set());
  // specs/0064 W2 — speakerKeys whose auto-label we have already reloaded the legend for.
  // The matcher re-derives the same auto-label every time a meeting is opened, so without
  // this the reload would re-trigger itself forever; a key is only acted on once.
  const appliedKeys = useRef<Set<string>>(new Set());

  // Seed dismissals from sessionStorage when the meeting changes (sticky per
  // meeting for the session). Best-effort: sessionStorage may be unavailable.
  useEffect(() => {
    appliedKeys.current = new Set();
    if (!meetingId) {
      setDismissedKeys(new Set());
      return;
    }
    const next = new Set<string>();
    try {
      const prefix = `${SUGGESTION_DISMISS_PREFIX}:${meetingId}:`;
      for (let i = 0; i < sessionStorage.length; i++) {
        const k = sessionStorage.key(i);
        if (k && k.startsWith(prefix) && sessionStorage.getItem(k) === 'true') {
          next.add(k.slice(prefix.length));
        }
      }
    } catch {
      /* sessionStorage unavailable — treat as nothing dismissed */
    }
    setDismissedKeys(next);
  }, [meetingId]);

  // The legend's own rows. Extracted from `refresh` so the suggestion path can reload just
  // the speakers after an auto-label lands, without re-running the whole fetch (which would
  // re-enter this and loop).
  const loadSpeakers = useCallback(async (id: string) => {
    try {
      const fetchedSpeakers = await invoke<MeetingSpeaker[]>(
        'api_get_meeting_speakers',
        { meetingId: id },
      );
      setSpeakers(fetchedSpeakers ?? []);
    } catch (error) {
      console.error('Failed to fetch meeting speakers:', error);
      setSpeakers([]);
    }
  }, []);

  // Merge a fresh batch of suggestions (from fetch or the diarization-complete
  // event), keyed by speakerKey. A new batch replaces the prior set entirely so
  // a re-diarization can retract stale suggestions.
  const applySuggestions = useCallback((list: SpeakerSuggestion[]) => {
    const map = new Map<string, SpeakerSuggestion>();
    // specs/0064 W2: an auto-labeled match has already been applied by the backend — the
    // name is on the speaker. Offering it as a chip would ask the user to confirm something
    // that is already done, which is the exact complaint this wave fixes.
    for (const s of list) if (!s.autoLabel) map.set(s.speakerKey, s);
    setRawSuggestions(map);
  }, []);

  // Best-effort fetch; a failure must never break the legend.
  //
  // The command applies the matches confident enough to need no confirmation before it
  // returns (specs/0064 W2), so a freshly auto-labeled speaker's name is in the database but
  // not yet in `speakers` — reload it, once per newly-applied key.
  const fetchSuggestions = useCallback(
    async (id: string) => {
      try {
        const list =
          (await invoke<SpeakerSuggestion[]>('api_get_speaker_suggestions', {
            meetingId: id,
          })) ?? [];
        applySuggestions(list);

        const fresh = list
          .filter((s) => s.autoLabel)
          .map((s) => s.speakerKey)
          .filter((key) => !appliedKeys.current.has(key));
        if (fresh.length > 0) {
          for (const key of fresh) appliedKeys.current.add(key);
          await loadSpeakers(id);
        }
      } catch (error) {
        console.error('Failed to fetch speaker suggestions:', error);
        // Leave any existing suggestions in place; do not surface to the user.
      }
    },
    [applySuggestions, loadSpeakers],
  );

  const refresh = useCallback(async () => {
    if (!meetingId) {
      setSpeakers([]);
      setAttendees([]);
      setPeople([]);
      setSuggestion(null);
      setRawSuggestions(new Map());
      return;
    }
    setIsLoading(true);

    // Cross-meeting suggestions are best-effort and independent of the rest;
    // fetch them in parallel without blocking the legend on a failure.
    void fetchSuggestions(meetingId);

    // People are a global directory (specs/0016 1b), offered as an additional
    // assign source. Ranked (specs/0038 WS5.a) so starred + frequent collaborators
    // float to the top of the speaker-identify pick-list. Best-effort: a failure
    // must never break the legend.
    void (async () => {
      try {
        const list = await invoke<Person[]>('api_list_people_ranked');
        setPeople(list ?? []);
      } catch (error) {
        console.error('Failed to fetch people:', error);
        setPeople([]);
      }
    })();
    await loadSpeakers(meetingId);

    // Attendees are best-effort: a meeting may have no linked calendar event, in
    // which case the roster is empty. Never surface that as an error.
    try {
      const roster = await invoke<MeetingAttendeesResponse>(
        'api_get_meeting_attendees',
        { meetingId },
      );
      // Distribution lists are not people (specs/0038 WS3): exclude them from the
      // speaker-assign pick-lists so a "Speaker N" can't be bound to a group address.
      // The DL floor + "Add members" affordance lives in ParticipantsPanel instead.
      setAttendees((roster?.attendees ?? []).filter((a) => !a.isDistributionList));
      setSuggestion(roster?.suggestion ?? null);
    } catch (error) {
      console.error('Failed to fetch meeting attendees:', error);
      setAttendees([]);
      setSuggestion(null);
    } finally {
      setIsLoading(false);
    }
  }, [meetingId, fetchSuggestions, loadSpeakers]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // specs/0078 — the owner is no longer structural in a room recording. Refreshed on
  // `diarization-complete` through the listener below. The meeting view's ONLY instance:
  // the transcript's "…" submenu gets `audioSetup` / `setAudioSetup` from here as props.
  const {
    setup: audioSetup,
    setOverride: setAudioSetup,
    refetch: refetchAudioSetup,
    applyDiarizationComplete: applyAudioSetupComplete,
  } = useAudioSetup(meetingId);

  // A diarization pass (re)assigns speakers; refresh the legend when one completes
  // for this meeting so newly-found speakers appear without a manual reload. The
  // offline pass also attaches cross-meeting `suggestions` to the event payload
  // (specs/0016 1a) — apply them immediately so "Looks like …" chips appear right
  // after "Identify speakers", rather than waiting on the parallel fetch. The same payload
  // carries the pass's audio setup (specs/0078), so this is the view's one listener.
  useEffect(() => {
    if (!meetingId) return;
    const dispose = safeListen<
      DiarizationCompleteSetupPayload & {
        meeting_id: string;
        suggestions?: SpeakerSuggestion[];
      }
    >('diarization-complete', (event) => {
      if (event.payload.meeting_id === meetingId) {
        applyAudioSetupComplete(event.payload);
        if (Array.isArray(event.payload.suggestions)) {
          applySuggestions(event.payload.suggestions);
        }
        void refresh();
      }
    });
    return () => dispose();
  }, [meetingId, refresh, applySuggestions, applyAudioSetupComplete]);

  // Re-fetch our speakers + refresh the transcript labels after a mutation.
  const afterMutation = useCallback(async () => {
    await refresh();
    await Promise.resolve(onMutated?.());
  }, [refresh, onMutated]);

  const renameSpeaker = useCallback(
    async (speakerKey: string, displayName: string) => {
      if (!meetingId) return;
      const trimmed = displayName.trim();
      if (!trimmed) return;
      try {
        await invoke('api_rename_speaker', {
          meetingId,
          speakerKey,
          displayName: trimmed,
        });
        await afterMutation();
        toast.success('Speaker renamed');
      } catch (error) {
        console.error('Failed to rename speaker:', error);
        toast.error('Could not rename speaker', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [meetingId, afterMutation],
  );

  const dismissSuggestion = useCallback(
    (speakerKey: string) => {
      setDismissedKeys((prev) => {
        if (prev.has(speakerKey)) return prev;
        const next = new Set(prev);
        next.add(speakerKey);
        return next;
      });
      if (!meetingId) return;
      try {
        sessionStorage.setItem(
          suggestionDismissKey(meetingId, speakerKey),
          'true',
        );
      } catch {
        /* sessionStorage unavailable — dismissal is in-memory only */
      }
    },
    [meetingId],
  );

  const assignAttendee = useCallback(
    async (speakerKey: string, attendee: { name: string; email: string }) => {
      if (!meetingId) return;
      try {
        await invoke('api_assign_speaker_to_attendee', {
          meetingId,
          speakerKey,
          displayName: attendee.name,
          email: attendee.email,
        });
        await afterMutation();
        // specs/0019 WS2.2 (note 7): assigning a known identity resolves the speaker —
        // clear any lingering "looks like X" cross-meeting suggestion for it so it
        // doesn't keep asking for confirmation.
        dismissSuggestion(speakerKey);
        // specs/0038 WS6.c: the backend also rosters this person as a participant —
        // signal the ParticipantsPanel to refresh so they appear without a reload.
        notifyMeetingParticipantsChanged(meetingId);
        toast.success(`Named as ${attendee.name}`);
      } catch (error) {
        console.error('Failed to assign attendee:', error);
        toast.error('Could not assign attendee', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [meetingId, afterMutation, dismissSuggestion],
  );

  const assignPerson = useCallback(
    async (speakerKey: string, person: Person) => {
      if (!meetingId) return;
      try {
        await invoke('api_assign_speaker_to_person', {
          meetingId,
          speakerKey,
          personId: person.id,
        });
        await afterMutation();
        // specs/0019 WS2.2 (note 7): no "looks like X" confirmation after a direct
        // assignment — dismiss the speaker's cross-meeting suggestion.
        dismissSuggestion(speakerKey);
        // specs/0038 WS6.c: the backend also rosters this person as a participant —
        // signal the ParticipantsPanel to refresh so they appear without a reload.
        notifyMeetingParticipantsChanged(meetingId);
        toast.success(`Named as ${person.displayName}`);
      } catch (error) {
        console.error('Failed to assign person:', error);
        toast.error('Could not assign person', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [meetingId, afterMutation, dismissSuggestion],
  );

  const mergeSpeakers = useCallback(
    async (fromKey: string, intoKey: string) => {
      if (!meetingId || fromKey === intoKey) return;
      try {
        await invoke('api_merge_speakers', {
          meetingId,
          fromKey,
          intoKey,
        });
        await afterMutation();
        toast.success('Speakers merged');
      } catch (error) {
        console.error('Failed to merge speakers:', error);
        toast.error('Could not merge speakers', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [meetingId, afterMutation],
  );

  const markAsMe = useCallback(
    async (speakerKeys: string | string[]) => {
      if (!meetingId) return;
      const keys = Array.isArray(speakerKeys) ? speakerKeys : [speakerKeys];
      try {
        // Sequential: the first creates "You", the rest merge into it.
        for (const speakerKey of keys) {
          await invoke('api_mark_speaker_as_me', { meetingId, speakerKey });
        }
        await afterMutation();
        toast.success('Marked as you');
      } catch (error) {
        console.error('Failed to mark speaker as you:', error);
        // A partial run (several keys) may still have changed rows; show them.
        await afterMutation().catch(() => {});
        toast.error('Could not mark this speaker as you', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [meetingId, afterMutation],
  );

  const unmarkMe = useCallback(async () => {
    if (!meetingId) return;
    try {
      await invoke('api_unmark_speaker_as_me', { meetingId });
      await afterMutation();
      toast.success('No longer marked as you');
    } catch (error) {
      console.error('Failed to unmark you:', error);
      toast.error('Could not change this speaker', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  }, [meetingId, afterMutation]);

  // Exposed suggestions = raw suggestions minus dismissed keys.
  const crossMeetingSuggestions = useMemo(() => {
    if (dismissedKeys.size === 0) return rawSuggestions;
    const map = new Map<string, SpeakerSuggestion>();
    for (const [key, value] of rawSuggestions) {
      if (!dismissedKeys.has(key)) map.set(key, value);
    }
    return map;
  }, [rawSuggestions, dismissedKeys]);

  const speakerPhotos = useMemo(
    () => buildSpeakerPhotoMap(speakers, attendees, people),
    [speakers, attendees, people],
  );

  return {
    speakers,
    attendees,
    people,
    suggestion,
    crossMeetingSuggestions,
    dismissSuggestion,
    isLoading,
    refresh,
    renameSpeaker,
    assignAttendee,
    assignPerson,
    mergeSpeakers,
    audioSetup,
    setAudioSetup,
    refetchAudioSetup,
    markAsMe,
    unmarkMe,
    speakerPhotos,
  };
}
