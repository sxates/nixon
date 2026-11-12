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
 *   3. Voice samples  — the reusable `VoiceprintSamplesList`, or a friendly opt-out state.
 */

import { Suspense, useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { InlineEditable } from '@/components/People/InlineEditable';
import { PersonActionsMenu } from '@/components/People/PersonActionsMenu';
import { VoiceprintSamplesList } from '@/components/People/VoiceprintSamplesList';
import { ForgetPersonDialog } from '@/app/people/ForgetPersonDialog';
import { Avatar } from '@/components/ui/avatar';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { ArrowLeft, Sparkles } from '@/components/ui/icons';
import { LinkRow } from '@/components/ui/list-row';
import { Panel } from '@/components/ui/panel';
import { RunStatus } from '@/components/ui/run-status';
import { Spinner } from '@/components/ui/spinner';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
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
    return <PageLoading className="bg-background" />;
  }

  if (!person) {
    return (
      <div className="flex h-page items-center justify-center bg-background px-8">
        <EmptyState
          title="That person no longer exists."
          action={
            <Button variant="outline" onClick={() => router.push('/people')}>
              <DeckIcon icon={ArrowLeft} size={14} />
              Back to People
            </Button>
          }
        />
      </div>
    );
  }

  const name = person.displayName?.trim() || 'Unnamed person';
  const citedSources = rollup?.sources.filter((s) => s.cited) ?? [];

  return (
    <div className="flex h-page flex-col bg-background animate-fade-in">
      {/* The shared page header (specs/0057 Task 2). The title carries the avatar and the
          inline name editor; the role editor is the SUBTITLE, outside the h1, so it keeps
          its own weight. The avatar's negative bottom margin lets it hang beside the
          subtitle (which is indented past it) instead of pushing the subtitle below it. */}
      <PageHeader
        back={{ label: 'People', onClick: onBack }}
        title={
          <span className="flex min-w-0 items-start gap-4">
            {/* Decorative inside the h1 — the avatar renders the person's initials,
                which would otherwise stutter the heading's accessible name. */}
            <span aria-hidden="true" className="-mb-7 flex-shrink-0">
              <Avatar
                name={name}
                photo={person.photoDataUri}
                size={56}
                colorClass={avatarColorClass(person.id)}
              />
            </span>
            <span className="min-w-0 flex-1 pt-1">
              <InlineEditable
                value={person.displayName}
                placeholder="Unnamed person"
                ariaLabel="Person name"
                allowEmpty={false}
                truncate={false}
                inputClassName="u-page-title text-title md:text-title"
                onCommit={(next) => void savePersonField({ displayName: next })}
              />
            </span>
          </span>
        }
        subtitle={
          <div className="pl-[4.5rem]">
            <InlineEditable
              value={person.role ?? ''}
              placeholder="Add a title"
              ariaLabel="Person title"
              allowEmpty
              inputClassName="text-meta md:text-meta"
              onCommit={(next) => void savePersonField({ role: next || null })}
            />
          </div>
        }
        actions={
          <PersonActionsMenu
            size="md"
            voiceprintOptOut={person.voiceprintOptOut}
            onToggleVoiceOptOut={() => void toggleOptOut()}
            optOutSaving={optOutSaving}
            onForget={() => setForgetOpen(true)}
          />
        }
      />

      <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
        <div className={cn('mx-auto', PAGE_CONTENT_WIDTH)}>
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
              inputClassName="text-sm md:text-sm leading-relaxed"
              onCommit={(next) => void savePersonField({ notes: next || null })}
            />
          </section>

          <Tabs value={tab} onValueChange={(v) => setTab(v as TabId)}>
            <TabsList>
              <TabsTrigger value="summary">Summary</TabsTrigger>
              <TabsTrigger value="meetings">Recent meetings</TabsTrigger>
              <TabsTrigger value="voice">Voice samples</TabsTrigger>
            </TabsList>

            {/* ── Summary — on-demand roll-up, cached in page state. ──────────── */}
            <TabsContent value="summary" className="mt-5">
              {rollupRunning ? (
                <RunStatus label="Summarizing recent activity…" />
              ) : rollupError ? (
                <Callout
                  tone="error"
                  title={rollupError}
                  action={
                    <Button variant="outline" size="sm" onClick={() => void runRollup()}>
                      Try again
                    </Button>
                  }
                >
                  Check your model settings, then try again.
                </Callout>
              ) : rollup ? (
                <Panel size="document">
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
                          <LinkRow
                            key={s.meetingId}
                            title={s.title?.trim() || 'Untitled meeting'}
                            meta={formatMeetingDate(s.createdAt) || undefined}
                            onClick={() => openMeeting(s.meetingId)}
                            hint="Open this meeting"
                          />
                        ))}
                      </div>
                    </div>
                  )}
                </Panel>
              ) : (
                <EmptyState
                  variant="panel"
                  titleAs="h3"
                  title="Summarize recent activity"
                  body={`Reads your last few meetings with ${name}. May take a moment on local models.`}
                  action={
                    <Button variant="brand" onClick={() => void runRollup()}>
                      <DeckIcon icon={Sparkles} size={14} />
                      Summarize
                    </Button>
                  }
                />
              )}
            </TabsContent>

            {/* ── Recent meetings — fast, no-LLM list. ────────────────────────── */}
            <TabsContent value="meetings" className="mt-5">
              {meetingsError ? (
                <Callout tone="error">{meetingsError}</Callout>
              ) : meetings === null ? (
                <p className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Spinner />
                  <span>Loading meetings…</span>
                </p>
              ) : meetings.length === 0 ? (
                <EmptyState variant="panel" titleAs="h3" title={`No meetings with ${name} yet.`} />
              ) : (
                <div className="flex flex-col gap-0.5">
                  {meetings.map((m) => (
                    <LinkRow
                      key={m.id}
                      title={m.title?.trim() || 'Untitled meeting'}
                      meta={formatMeetingDate(m.startedAt) || undefined}
                      onClick={() => openMeeting(m.id)}
                      hint="Open this meeting"
                    />
                  ))}
                </div>
              )}
            </TabsContent>

            {/* ── Voice samples — reusable list, or opt-out state. ─────────────── */}
            <TabsContent value="voice" className="mt-5">
              {person.voiceprintOptOut ? (
                <EmptyState
                  variant="panel"
                  titleAs="h3"
                  title="Voice isn't stored"
                  body={`You've asked Nixon not to store ${name}'s voice, so there are no samples here. You can change this from the menu above.`}
                />
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
    </div>
  );
}

export default function PersonDetailsPage() {
  // useSearchParams requires a Suspense boundary (same pattern as /meeting-details, /ask).
  return (
    <Suspense fallback={<PageLoading className="bg-background" />}>
      <PersonDetailsContent />
    </Suspense>
  );
}
