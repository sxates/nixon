'use client';

/**
 * PrepPanel (specs/0036) — the Prep tab's body in meeting-details.
 *
 * Three stacked sections for an upcoming (or already-recorded) occurrence:
 *   1. Brief — a short synthesized "where we left off / decisions / open threads"
 *      over the series' prior occurrences, rendered with AnswerMarkdown so its
 *      `[M#]` citations become chips linking to the source meetings. Pre-generated
 *      in the background, so it's usually instant; while pending it shows a spinner
 *      and fills in over the `prep-brief-*` events (all filtered by meetingId).
 *   2. Carried-over open action items — still-open commitments from prior
 *      occurrences, split into "yours" vs. "owed by others" (read-only here; the
 *      mutable surface is the Action items section / task hub).
 *   3. Prep notes — the editable agenda (PrepNotesEditor).
 *
 * Event discipline mirrors Ask AI: listeners live for the component's lifetime and
 * every payload is dropped unless its `meetingId` matches ours. `prep-briefs-updated`
 * (a payload-less background broadcast) triggers a re-read of the cached view.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { ChevronRight, Link2, Loader2, RefreshCw, Sparkles, Unlink } from 'lucide-react';
import { toast } from 'sonner';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { formatMeetingDate } from '@/lib/format-date';
import { assigneeLabel } from '@/lib/action-items';
import { safeListen } from '@/lib/safe-listen';
import {
  isBriefLoading,
  prepStageLabel,
  splitOpenItems,
  type BriefStatus,
  type PrepBriefCompletePayload,
  type PrepBriefErrorPayload,
  type PrepBriefProgressPayload,
  type PrepStage,
  type PrepView,
  type SourceMeeting,
} from '@/lib/prep';
import type { ActionItem, Person } from '@/types';
import { cn } from '@/lib/utils';
import { LinkMeetingPicker } from './LinkMeetingPicker';
import { PrepNotesEditor } from './PrepNotesEditor';

export function PrepPanel({ meetingId }: { meetingId: string }) {
  const router = useRouter();

  const [view, setView] = useState<PrepView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Brief state is tracked separately from `view` so the streaming `prep-brief-*`
  // events can update it without disturbing the loaded openItems / prep notes.
  const [briefStatus, setBriefStatus] = useState<BriefStatus>('absent');
  const [briefMarkdown, setBriefMarkdown] = useState<string | null>(null);
  const [briefSources, setBriefSources] = useState<SourceMeeting[]>([]);
  const [briefError, setBriefError] = useState<string | null>(null);
  const [stage, setStage] = useState<PrepStage | null>(null);

  const [people, setPeople] = useState<Person[]>([]);

  // Manual series association (specs/0041 WS4): the "Link previous meeting…" picker.
  const [linkPickerOpen, setLinkPickerOpen] = useState(false);
  const [unlinkingId, setUnlinkingId] = useState<string | null>(null);

  // Guard against a `prep-briefs-updated` refetch clobbering fresher event state.
  const briefFromEventRef = useRef(false);

  const applyView = useCallback((v: PrepView) => {
    setView(v);
    // Only adopt the persisted brief when a live event hasn't already moved us
    // past it (e.g. a background broadcast races an in-flight completion).
    if (!briefFromEventRef.current) {
      setBriefStatus(v.briefStatus);
      setBriefMarkdown(v.briefMarkdown);
      setBriefSources(v.briefSources ?? []);
      setBriefError(null);
    }
  }, []);

  // Initial load (+ re-read on the background broadcast).
  const loadPrep = useCallback(async () => {
    try {
      const v = await invoke<PrepView>('api_get_prep', { meetingId });
      applyView(v);
      setLoadError(null);
    } catch (err) {
      console.error('Failed to load prep view:', err);
      setLoadError('We could not load the prep for this meeting. Please try again.');
    }
  }, [meetingId, applyView]);

  useEffect(() => {
    void loadPrep();
  }, [loadPrep]);

  // People directory — to resolve "owed by others" assignee display names.
  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<Person[]>('api_list_people');
        setPeople(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('Failed to load people for prep:', err);
      }
    })();
  }, []);

  // Lifetime brief-event subscriptions, every payload filtered by meetingId.
  useEffect(() => {
    const disposers = [
      safeListen<PrepBriefProgressPayload>('prep-brief-progress', (e) => {
        if (e.payload.meetingId !== meetingId) return;
        briefFromEventRef.current = true;
        setStage(e.payload.stage);
        setBriefStatus('pending');
      }),
      safeListen<PrepBriefCompletePayload>('prep-brief-complete', (e) => {
        if (e.payload.meetingId !== meetingId) return;
        briefFromEventRef.current = true;
        setStage(null);
        setBriefStatus(e.payload.status);
        setBriefMarkdown(e.payload.markdown);
        setBriefSources(e.payload.sources ?? []);
        setBriefError(null);
      }),
      safeListen<PrepBriefErrorPayload>('prep-brief-error', (e) => {
        if (e.payload.meetingId !== meetingId) return;
        briefFromEventRef.current = true;
        setStage(null);
        setBriefStatus('failed');
        setBriefError(e.payload.message);
      }),
      // Background pass changed some brief — re-read our cached view. The event
      // guard is cleared so the fresh persisted brief is adopted.
      safeListen('prep-briefs-updated', () => {
        briefFromEventRef.current = false;
        void loadPrep();
      }),
    ];
    return () => disposers.forEach((dispose) => dispose());
  }, [meetingId, loadPrep]);

  const regenerate = useCallback(async () => {
    briefFromEventRef.current = true;
    setBriefStatus('pending');
    setBriefMarkdown(null);
    setBriefError(null);
    setStage(null);
    try {
      await invoke('api_regenerate_prep_brief', { meetingId });
    } catch (err) {
      console.error('Failed to regenerate prep brief:', err);
      setBriefStatus('failed');
      setBriefError('Could not start a refresh. Check your model settings and try again.');
    }
  }, [meetingId]);

  const nameById = useMemo(
    () => new Map(people.map((p) => [p.id, p.displayName])),
    [people],
  );

  const { mine, others } = useMemo(
    () => splitOpenItems(view?.openItems ?? []),
    [view?.openItems],
  );

  // Group "owed by others" by assignee, resolving people → display names via the SHARED
  // assigneeLabel helper (consistent 'Me'/'Unknown'/raw fallbacks with the Action items list).
  const otherGroups = useMemo(() => {
    const map = new Map<string, { label: string; items: ActionItem[] }>();
    for (const it of others) {
      const key = it.assigneePersonId ?? (it.assigneeRaw ? `raw:${it.assigneeRaw}` : 'unassigned');
      const label = assigneeLabel(it, nameById) ?? 'Unassigned';
      const group = map.get(key);
      if (group) group.items.push(it);
      else map.set(key, { label, items: [it] });
    }
    return Array.from(map.values());
  }, [others, nameById]);

  const openMeeting = useCallback(
    (id: string) => router.push(`/meeting-details?id=${id}`),
    [router],
  );

  // After a link lands the backend regenerates the brief and re-broadcasts; re-read the
  // view (clearing the event guard so the fresh persisted state is adopted).
  const handleLinked = useCallback(() => {
    briefFromEventRef.current = false;
    void loadPrep();
  }, [loadPrep]);

  const unlinkMeeting = useCallback(
    async (id: string) => {
      setUnlinkingId(id);
      try {
        await invoke('api_unlink_meeting_from_series', { meetingId: id });
        briefFromEventRef.current = false;
        await loadPrep();
      } catch (err) {
        console.error('Failed to unlink meeting from series:', err);
        toast.error('We could not unlink that meeting. Please try again.');
      } finally {
        setUnlinkingId(null);
      }
    },
    [loadPrep],
  );

  const citedSources = briefSources.filter((s) => s.cited);

  if (loadError) {
    return (
      <div className="rounded-[3px] border border-destructive/30 bg-destructive/5 px-4 py-6 text-center">
        <p className="text-sm text-foreground">{loadError}</p>
        <button
          type="button"
          onClick={() => void loadPrep()}
          className="mt-3 rounded-lg border border-border bg-card px-3 py-1.5 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Try again
        </button>
      </div>
    );
  }

  if (!view) {
    return (
      <div className="flex h-40 items-center justify-center text-muted-foreground">
        <Loader2 className="mr-2 h-5 w-5 animate-spin" />
        <span className="text-sm">Loading prep…</span>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-8">
      {/* ── Brief ─────────────────────────────────────────────────────────── */}
      <section aria-label="Pre-call brief">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h2 className="u-section-label flex items-center gap-1.5">
            <Sparkles size={13} aria-hidden="true" />
            Before this meeting
          </h2>
          <div className="flex items-center gap-1.5">
            {briefStatus !== 'none' && (
              <button
                type="button"
                onClick={() => setLinkPickerOpen(true)}
                title="Mark a past meeting as a previous occurrence of this one"
                className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 py-1 text-xs font-semibold text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <Link2 size={12} aria-hidden="true" />
                Link previous meeting…
              </button>
            )}
            {(briefStatus === 'ready' || briefStatus === 'failed') && (
              <button
                type="button"
                onClick={() => void regenerate()}
                title="Regenerate the brief"
                className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 py-1 text-xs font-semibold text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <RefreshCw size={12} aria-hidden="true" />
                Regenerate
              </button>
            )}
          </div>
        </div>

        {isBriefLoading(briefStatus) ? (
          <div
            role="status"
            className="flex items-center gap-2 rounded-[3px] border border-border bg-card px-4 py-4 text-sm text-foreground shadow-sm"
          >
            <Loader2 size={15} aria-hidden="true" className="animate-spin text-brand" />
            <span className="truncate">
              {stage ? prepStageLabel(stage) : 'Preparing your brief…'}
            </span>
          </div>
        ) : briefStatus === 'none' ? (
          <div className="rounded-[3px] border border-dashed border-border bg-card/50 px-4 py-6 text-center">
            <p className="text-sm text-muted-foreground">
              No previous meetings found for this series. If an earlier meeting belongs
              here — recorded ad-hoc or under a different name — link it and a brief will
              be generated from it.
            </p>
            <button
              type="button"
              onClick={() => setLinkPickerOpen(true)}
              className="mt-3 inline-flex items-center gap-1.5 rounded-lg border border-border bg-card px-3 py-1.5 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <Link2 size={12} aria-hidden="true" />
              Link previous meeting…
            </button>
          </div>
        ) : briefStatus === 'failed' ? (
          <div className="rounded-[3px] border border-destructive/30 bg-destructive/5 px-4 py-4">
            <p className="text-sm text-foreground">
              {briefError ?? 'The brief could not be generated.'}
            </p>
            <p className="u-meta mt-1">Try Regenerate, or check your model settings.</p>
          </div>
        ) : briefMarkdown ? (
          <div className="rounded-[3px] border border-border bg-card p-5 shadow-sm">
            <AnswerMarkdown markdown={briefMarkdown} sources={briefSources} />
            {citedSources.length > 0 && (
              <div className="mt-5 border-t border-border pt-3">
                <h3 className="u-section-label">From</h3>
                <div className="mt-1 flex flex-col gap-0.5">
                  {citedSources.map((s) => (
                    <button
                      key={s.meetingId}
                      type="button"
                      onClick={() => openMeeting(s.meetingId)}
                      title="Open this meeting"
                      className="group/source flex items-baseline gap-2 rounded px-2 py-1 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span className="min-w-0 truncate text-[13.5px] font-semibold text-foreground group-hover/source:text-brand">
                        {s.title?.trim() || 'Untitled meeting'}
                      </span>
                      {formatMeetingDate(s.createdAt) && (
                        <span className="flex-shrink-0 text-xs text-muted-foreground">
                          {formatMeetingDate(s.createdAt)}
                        </span>
                      )}
                      <ChevronRight
                        size={13}
                        aria-hidden="true"
                        className="self-center text-muted-foreground group-hover/source:text-brand"
                      />
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="rounded-[3px] border border-dashed border-border bg-card/50 px-4 py-6 text-center">
            <p className="text-sm text-muted-foreground">No brief available.</p>
          </div>
        )}

        {/* Manually linked meetings (specs/0041 WS4), with an unlink affordance. */}
        {(view.linkedMeetings ?? []).length > 0 && (
          <div className="mt-3">
            <h3 className="u-section-label">Linked meetings</h3>
            <ul className="mt-1 flex flex-col gap-0.5">
              {(view.linkedMeetings ?? []).map((m) => (
                <li key={m.id} className="group/linked flex items-center gap-2 rounded px-2 py-1">
                  <button
                    type="button"
                    onClick={() => openMeeting(m.id)}
                    title="Open this meeting"
                    className="flex min-w-0 flex-1 items-baseline gap-2 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <span className="min-w-0 truncate text-[13.5px] font-semibold text-foreground hover:text-brand">
                      {m.title?.trim() || 'Untitled meeting'}
                    </span>
                    {formatMeetingDate(m.startedAt) && (
                      <span className="flex-shrink-0 text-xs text-muted-foreground">
                        {formatMeetingDate(m.startedAt)}
                      </span>
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => void unlinkMeeting(m.id)}
                    disabled={unlinkingId !== null}
                    title="Unlink this meeting from the series"
                    aria-label={`Unlink ${m.title?.trim() || 'Untitled meeting'}`}
                    className="inline-flex flex-shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-xs font-semibold text-muted-foreground transition-colors hover:text-destructive focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                  >
                    {unlinkingId === m.id ? (
                      <Loader2 size={12} aria-hidden="true" className="animate-spin" />
                    ) : (
                      <Unlink size={12} aria-hidden="true" />
                    )}
                    Unlink
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      <LinkMeetingPicker
        open={linkPickerOpen}
        onOpenChange={setLinkPickerOpen}
        targetMeetingId={meetingId}
        excludeIds={[meetingId, ...(view.linkedMeetings ?? []).map((m) => m.id)]}
        onLink={handleLinked}
      />

      {/* ── Carried-over open items ───────────────────────────────────────── */}
      {(mine.length > 0 || otherGroups.length > 0) && (
        <section aria-label="Carried-over open items" className="flex flex-col gap-5">
          <div>
            <h2 className="u-section-label">Your open items</h2>
            {mine.length > 0 ? (
              <ul className="mt-2 flex flex-col gap-1.5">
                {mine.map((item) => (
                  <OpenItemRow key={item.id} item={item} onOpenMeeting={openMeeting} />
                ))}
              </ul>
            ) : (
              <p className="u-meta mt-2">Nothing outstanding on your side.</p>
            )}
          </div>

          {otherGroups.length > 0 && (
            <div>
              <h2 className="u-section-label">Owed by others</h2>
              <div className="mt-2 flex flex-col gap-3">
                {otherGroups.map((group) => (
                  <div key={group.label}>
                    <p className="px-1 text-[12.5px] font-semibold text-foreground">
                      {group.label}
                    </p>
                    <ul className="mt-1 flex flex-col gap-1.5">
                      {group.items.map((item) => (
                        <OpenItemRow key={item.id} item={item} onOpenMeeting={openMeeting} />
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            </div>
          )}
        </section>
      )}

      {/* ── Prep notes (agenda) ───────────────────────────────────────────── */}
      <section aria-label="Prep notes">
        <PrepNotesEditor
          meetingId={meetingId}
          initialMarkdown={view.prepNotesMarkdown}
          initialJson={view.prepNotesJson}
        />
      </section>
    </div>
  );
}

/** Read-only carried-over open item. Links to its source occurrence. */
function OpenItemRow({
  item,
  onOpenMeeting,
}: {
  item: ActionItem;
  onOpenMeeting: (meetingId: string) => void;
}) {
  return (
    <li className="rounded-lg border border-border bg-card px-3 py-2">
      <p className="text-[14px] leading-snug text-foreground">{item.description}</p>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
        {item.dueHint && <span>{item.dueHint}</span>}
        {item.dueHint && item.meetingId && <span aria-hidden="true">·</span>}
        {item.meetingId && (
          <button
            type="button"
            onClick={() => onOpenMeeting(item.meetingId!)}
            title="Open the meeting this came from"
            className={cn(
              'inline-flex items-center gap-0.5 rounded font-semibold text-muted-foreground',
              'transition-colors hover:text-brand focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
            )}
          >
            From this meeting
            <ChevronRight size={12} aria-hidden="true" />
          </button>
        )}
      </div>
    </li>
  );
}

