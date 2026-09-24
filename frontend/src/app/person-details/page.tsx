'use client';

/**
 * Person-detail page (specs/0038 dogfood feedback #3).
 *
 * A dedicated page for one person — the query-param `Page`/`PageContent` + `<Suspense>`
 * shape shared with `/meeting-details`, `/ask`, and `/saved-question`. It replaces the old
 * inline `PersonDetailPanel` expansion in the People directory.
 *
 * Header: avatar (directory photo or initials, specs/0056 W6) + INLINE-editable name & role/title (click-to-edit, save on
 * Enter/blur via `api_update_person`, optimistic with revert + toast on failure), a
 * history-aware back button, and an overflow menu (voice opt-out toggle + "forget").
 *
 * Tabs (default Summary):
 *   1. Summary        — on-demand LLM roll-up (`api_person_rollup`), cached in page state
 *                       so switching tabs never re-runs the slow local model.
 *   2. Recent meetings — the fast, no-LLM `api_recent_meetings_with_person` list.
 *   3. Voice Samples  — the reusable `VoiceprintSamplesList`, or a friendly opt-out state.
 */

import { Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  ArrowLeft,
  Check,
  ChevronRight,
  Loader2,
  MicOff,
  MoreHorizontal,
  Pencil,
  Sparkles,
  Trash2,
} from 'lucide-react';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { PersonAvatar } from '@/components/People/PersonAvatar';
import { VoiceprintSamplesList } from '@/components/People/VoiceprintSamplesList';
import { ForgetPersonDialog } from '@/app/people/ForgetPersonDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { getPerson } from '@/lib/people';
import {
  fetchPersonRollup,
  fetchRecentMeetingsWithPerson,
  type PersonRollup,
  type RecentPersonMeeting,
} from '@/lib/person-rollup';
import { formatMeetingDate } from '@/lib/format-date';
import { cn } from '@/lib/utils';
import { avatarColorClass } from '@/lib/avatar-colors';
import type { Person } from '@/types';
import { PageHeader } from '@/components/ui/page-header';

/**
 * A click-to-edit text field. Renders as text with a hover pencil; clicking swaps to an
 * input that commits on Enter/blur and reverts on Escape. Empty commits are rejected unless
 * `allowEmpty` (name is required; role can be cleared).
 */
function InlineEditable({
  value,
  placeholder,
  ariaLabel,
  allowEmpty,
  textClassName,
  inputClassName,
  onCommit,
  multiline = false,
  truncate = true,
}: {
  value: string;
  placeholder: string;
  ariaLabel: string;
  allowEmpty: boolean;
  textClassName: string;
  inputClassName: string;
  onCommit: (next: string) => void;
  /** Render a wrapping textarea (notes) instead of a single-line input. */
  multiline?: boolean;
  /** Clip the display text to one line with an ellipsis. Off for the name/notes. */
  truncate?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Keep the draft synced with the source of truth while not actively editing.
  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  useEffect(() => {
    if (editing) {
      const id = window.setTimeout(() => {
        const el = multiline ? textareaRef.current : inputRef.current;
        el?.focus();
        el?.select();
      }, 0);
      return () => window.clearTimeout(id);
    }
  }, [editing, multiline]);

  const commit = useCallback(() => {
    setEditing(false);
    const trimmed = draft.trim();
    if (!allowEmpty && !trimmed) {
      setDraft(value); // reject empty — revert to current value
      return;
    }
    if (trimmed !== value.trim()) onCommit(trimmed);
  }, [draft, value, allowEmpty, onCommit]);

  if (editing) {
    // Escape reverts+closes for both; single-line commits on Enter, the textarea
    // keeps Enter for newlines and commits with ⌘/Ctrl+Enter (or blur).
    const onKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setDraft(value);
        setEditing(false);
      } else if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commit();
      }
    };
    if (multiline) {
      return (
        <textarea
          ref={textareaRef}
          value={draft}
          placeholder={placeholder}
          aria-label={ariaLabel}
          rows={3}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={onKeyDown}
          className={cn(
            'w-full resize-y rounded-md border border-input bg-muted px-2 py-1.5 text-foreground focus:outline-none focus:ring-2 focus:ring-ring',
            inputClassName,
          )}
        />
      );
    }
    return (
      <input
        ref={inputRef}
        type="text"
        value={draft}
        placeholder={placeholder}
        aria-label={ariaLabel}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
        className={cn(
          'w-full rounded-md border border-input bg-muted px-2 py-1 text-foreground focus:outline-none focus:ring-2 focus:ring-ring',
          inputClassName,
        )}
      />
    );
  }

  const display = value.trim() || placeholder;
  const isPlaceholder = !value.trim();
  return (
    <button
      type="button"
      onClick={() => setEditing(true)}
      title="Click to edit"
      aria-label={ariaLabel}
      className={cn(
        'group -ml-2 flex w-fit max-w-full gap-2 rounded-md px-2 py-1 text-left transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        multiline ? 'items-start' : 'items-center',
      )}
    >
      <span
        className={cn(
          truncate ? 'truncate' : 'whitespace-pre-wrap break-words',
          textClassName,
          isPlaceholder && 'text-muted-foreground',
        )}
      >
        {display}
      </span>
      <Pencil
        className={cn(
          'h-4 w-4 flex-shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100',
          multiline && 'mt-1',
        )}
      />
    </button>
  );
}

type TabId = 'summary' | 'meetings' | 'voice';

function PersonDetailsContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const id = searchParams.get('id');

  const [loading, setLoading] = useState(true);
  const [person, setPerson] = useState<Person | null>(null);
  const [tab, setTab] = useState<TabId>('summary');
  const [forgetOpen, setForgetOpen] = useState(false);
  const [optOutSaving, setOptOutSaving] = useState(false);

  // ── Recent meetings (cheap, no-LLM). ───────────────────────────────────────
  const [meetings, setMeetings] = useState<RecentPersonMeeting[] | null>(null);
  const [meetingsError, setMeetingsError] = useState<string | null>(null);

  // ── Roll-up (slow LLM) — on-demand, cached in page state for the session. ───
  const [rollup, setRollup] = useState<PersonRollup | null>(null);
  const [rollupRunning, setRollupRunning] = useState(false);
  const [rollupError, setRollupError] = useState<string | null>(null);

  // Load the person.
  useEffect(() => {
    let alive = true;
    void (async () => {
      if (!id) {
        setLoading(false);
        return;
      }
      try {
        const p = await getPerson(id);
        if (alive) setPerson(p);
      } catch (err) {
        console.error('Failed to load person:', err);
        if (alive) toast.error('Could not load this person.');
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [id]);

  // Load recent meetings once we have the person.
  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    setMeetings(null);
    setMeetingsError(null);
    void (async () => {
      try {
        const rows = await fetchRecentMeetingsWithPerson(id);
        if (!cancelled) setMeetings(Array.isArray(rows) ? rows : []);
      } catch (err) {
        console.error('Failed to load recent meetings with person:', err);
        if (!cancelled) setMeetingsError('Could not load recent meetings.');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id]);

  const runRollup = useCallback(async () => {
    if (!id || rollupRunning) return;
    setRollupRunning(true);
    setRollupError(null);
    try {
      const result = await fetchPersonRollup(id);
      setRollup(result);
    } catch (err) {
      console.error('Failed to synthesize person roll-up:', err);
      setRollupError(
        typeof err === 'string'
          ? err
          : err instanceof Error
            ? err.message
            : 'The roll-up could not be generated.',
      );
    } finally {
      setRollupRunning(false);
    }
  }, [id, rollupRunning]);

  const openMeeting = useCallback(
    (meetingId: string) => router.push(`/meeting-details?id=${meetingId}`),
    [router],
  );

  const onBack = useCallback(() => {
    if (window.history.length > 1) router.back();
    else router.push('/people');
  }, [router]);

  // Persist a field edit (name/role) via api_update_person; optimistic with revert.
  const savePersonField = useCallback(
    async (patch: Partial<Pick<Person, 'displayName' | 'role' | 'email' | 'notes'>>) => {
      if (!person) return;
      const prev = person;
      const next = { ...person, ...patch };
      setPerson(next);
      try {
        await invoke('api_update_person', {
          id: person.id,
          displayName: next.displayName.trim(),
          email: next.email?.trim() || null,
          role: next.role?.trim() || null,
          notes: next.notes?.trim() || null,
        });
      } catch (err) {
        console.error('Failed to update person:', err);
        setPerson(prev);
        toast.error('Could not save your change.', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [person],
  );

  // Flip the voiceprint opt-out flag; optimistic with revert.
  const toggleOptOut = useCallback(async () => {
    if (!person || optOutSaving) return;
    const next = !person.voiceprintOptOut;
    setOptOutSaving(true);
    setPerson((p) => (p ? { ...p, voiceprintOptOut: next } : p));
    try {
      await invoke('api_set_person_voiceprint_opt_out', { id: person.id, optOut: next });
    } catch (err) {
      console.error('Failed to update voiceprint setting:', err);
      setPerson((p) => (p ? { ...p, voiceprintOptOut: !next } : p));
      toast.error('Could not update voice setting.', {
        description: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setOptOutSaving(false);
    }
  }, [person, optOutSaving]);

  if (loading) {
    return (
      <div className="flex h-page items-center justify-center bg-background text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    );
  }

  if (!person) {
    return (
      <div className="flex h-page flex-col items-center justify-center gap-4 bg-background px-8 text-center">
        <p className="text-sm text-muted-foreground">That person no longer exists.</p>
        <button
          type="button"
          onClick={() => router.push('/people')}
          className="inline-flex h-9 items-center gap-1.5 rounded-[3px] border border-border bg-card px-3 text-sm font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          Back to People
        </button>
      </div>
    );
  }

  const name = person.displayName?.trim() || 'Unnamed person';
  const citedSources = rollup?.sources.filter((s) => s.cited) ?? [];

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      <div className="flex-shrink-0 px-4 min-[900px]:px-7 pt-7">
        <button
          type="button"
          onClick={onBack}
          className="u-meta inline-flex items-center gap-1.5 text-muted-foreground transition-colors hover:text-foreground focus:outline-none"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          People
        </button>
      </div>

      {/* The shared page header (specs/0057 Task 2). The title slot carries the
          avatar plus the inline name/role editors, so this page's identity block
          sits on the same baseline and padding as every other screen. */}
      <PageHeader
        className="pt-3"
        title={
          <span className="flex min-w-0 items-center gap-4">
            {/* Decorative inside the h1 — the avatar renders the person's initials,
                which would otherwise stutter the heading's accessible name. */}
            <span aria-hidden="true" className="flex-shrink-0">
              <PersonAvatar
                name={name}
                photoDataUri={person.photoDataUri}
                size="lg"
                colorClass={avatarColorClass(person.id)}
              />
            </span>
            <span className="min-w-0 flex-1">
              <InlineEditable
                value={person.displayName}
                placeholder="Unnamed person"
                ariaLabel="Person name"
                allowEmpty={false}
                truncate={false}
                textClassName="font-display text-[22px] font-semibold leading-[1.1] tracking-[-0.011em] text-foreground"
                inputClassName="font-display text-[22px] font-semibold tracking-[-0.011em]"
                onCommit={(next) => void savePersonField({ displayName: next })}
              />
              <InlineEditable
                value={person.role ?? ''}
                placeholder="Add a title"
                ariaLabel="Person title"
                allowEmpty
                textClassName="text-sm text-muted-foreground"
                inputClassName="text-sm"
                onCommit={(next) => void savePersonField({ role: next || null })}
              />
            </span>
          </span>
        }
        actions={
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label="Person options"
                title="Person options"
                className="inline-flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <MoreHorizontal className="h-5 w-5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                disabled={optOutSaving}
                onSelect={(e) => {
                  e.preventDefault();
                  void toggleOptOut();
                }}
              >
                <MicOff className="mr-2 h-4 w-4" />
                Don&apos;t store this person&apos;s voice
                {person.voiceprintOptOut && <Check className="ml-auto h-4 w-4" />}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onSelect={() => setForgetOpen(true)}
                className="text-destructive focus:text-destructive"
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Forget this person
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        }
      />

      <div className="flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12">
        <div className="mx-auto max-w-[840px]">
          {/* Notes — the free-text description of this person (role, how you know
              them). Restored on the detail page (specs/0038 feedback): the field
              lives on `Person.notes` and was previously only editable in the dialog. */}
          <section className="mb-6" aria-label="Notes">
            <h2 className="u-section-label mb-1">Notes</h2>
            <InlineEditable
              value={person.notes ?? ''}
              placeholder="Add notes about this person — their role, how you know them…"
              ariaLabel="Person notes"
              allowEmpty
              multiline
              truncate={false}
              textClassName="text-sm leading-relaxed text-foreground"
              inputClassName="text-sm leading-relaxed"
              onCommit={(next) => void savePersonField({ notes: next || null })}
            />
          </section>

          <Tabs value={tab} onValueChange={(v) => setTab(v as TabId)}>
            <TabsList>
              <TabsTrigger value="summary">Summary</TabsTrigger>
              <TabsTrigger value="meetings">Recent meetings</TabsTrigger>
              <TabsTrigger value="voice">Voice Samples</TabsTrigger>
            </TabsList>

            {/* ── Summary — on-demand roll-up, cached in page state. ──────────── */}
            <TabsContent value="summary" className="mt-5">
              {rollupRunning ? (
                <div
                  role="status"
                  className="flex items-center gap-2 rounded-[3px] border border-border bg-card px-4 py-4 text-sm text-foreground shadow-sm"
                >
                  <Loader2 size={15} aria-hidden="true" className="animate-spin text-brand" />
                  <span className="truncate">Summarizing recent activity…</span>
                </div>
              ) : rollupError ? (
                <div className="rounded-[3px] border border-destructive/30 bg-destructive/5 px-4 py-4">
                  <p className="text-sm text-foreground">{rollupError}</p>
                  <p className="u-meta mt-1">Check your model settings, then try again.</p>
                  <button
                    type="button"
                    onClick={() => void runRollup()}
                    className="mt-3 rounded-[3px] border border-border bg-card px-3 py-1.5 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Try again
                  </button>
                </div>
              ) : rollup ? (
                <div className="rounded-[3px] border border-border bg-card p-5 shadow-sm">
                  {rollup.markdown.trim() ? (
                    <AnswerMarkdown markdown={rollup.markdown} sources={rollup.sources} />
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      Nothing notable to summarize from recent meetings with {name}.
                    </p>
                  )}
                  {citedSources.length > 0 && (
                    <div className="mt-5 border-t border-border pt-3">
                      <h4 className="u-section-label">From</h4>
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
                <div className="rounded-[3px] border border-dashed border-border bg-card/50 px-6 py-10 text-center">
                  <Sparkles
                    size={20}
                    aria-hidden="true"
                    className="mx-auto mb-3 text-muted-foreground"
                  />
                  <p className="text-sm font-medium text-foreground">Summarize recent activity</p>
                  <p className="u-meta mx-auto mt-1 max-w-sm">
                    Reads your last few meetings with {name}. May take a moment on local models.
                  </p>
                  <button
                    type="button"
                    onClick={() => void runRollup()}
                    className="mt-4 inline-flex items-center gap-1.5 rounded-[3px] bg-brand px-4 py-2 text-sm font-semibold text-brand-foreground transition-colors hover:bg-brand/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <Sparkles size={14} aria-hidden="true" />
                    Summarize
                  </button>
                </div>
              )}
            </TabsContent>

            {/* ── Recent meetings — fast, no-LLM list. ────────────────────────── */}
            <TabsContent value="meetings" className="mt-5">
              {meetingsError ? (
                <p className="text-sm text-muted-foreground">{meetingsError}</p>
              ) : meetings === null ? (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                  <span>Loading meetings…</span>
                </div>
              ) : meetings.length === 0 ? (
                <p className="text-sm text-muted-foreground">No meetings with {name} yet.</p>
              ) : (
                <div className="flex flex-col gap-0.5">
                  {meetings.map((m) => (
                    <button
                      key={m.id}
                      type="button"
                      onClick={() => openMeeting(m.id)}
                      title="Open this meeting"
                      className="group/meeting flex items-baseline gap-2 rounded px-2 py-1.5 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span className="min-w-0 truncate text-[13.5px] font-semibold text-foreground group-hover/meeting:text-brand">
                        {m.title?.trim() || 'Untitled meeting'}
                      </span>
                      {formatMeetingDate(m.startedAt) && (
                        <span className="flex-shrink-0 text-xs text-muted-foreground">
                          {formatMeetingDate(m.startedAt)}
                        </span>
                      )}
                      <ChevronRight
                        size={13}
                        aria-hidden="true"
                        className="self-center text-muted-foreground group-hover/meeting:text-brand"
                      />
                    </button>
                  ))}
                </div>
              )}
            </TabsContent>

            {/* ── Voice Samples — reusable list, or opt-out state. ─────────────── */}
            <TabsContent value="voice" className="mt-5">
              {person.voiceprintOptOut ? (
                <div className="flex flex-col items-center justify-center rounded-[3px] border border-dashed border-border bg-card/50 px-6 py-10 text-center">
                  <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-muted">
                    <MicOff className="h-5 w-5 text-muted-foreground" />
                  </div>
                  <p className="text-sm font-medium text-foreground">Voice isn&apos;t stored</p>
                  <p className="u-meta mx-auto mt-1 max-w-xs">
                    You&apos;ve asked Nixon not to store {name}&apos;s voice, so there are no
                    samples here. You can change this from the menu above.
                  </p>
                </div>
              ) : (
                <VoiceprintSamplesList personId={person.id} personName={person.displayName} />
              )}
            </TabsContent>
          </Tabs>
        </div>
      </div>

      <ForgetPersonDialog
        open={forgetOpen}
        onOpenChange={setForgetOpen}
        personId={person.id}
        personName={person.displayName}
        onForgotten={() => router.push('/people')}
      />
    </motion.div>
  );
}

export default function PersonDetailsPage() {
  // useSearchParams requires a Suspense boundary (same pattern as /meeting-details, /ask).
  return (
    <Suspense
      fallback={
        <div className="flex h-page items-center justify-center bg-background text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      }
    >
      <PersonDetailsContent />
    </Suspense>
  );
}
