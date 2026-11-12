'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Avatar } from '@/components/ui/avatar';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { IconButton } from '@/components/ui/icon-button';
import { Search, Star } from '@/components/ui/icons';
import { Input } from '@/components/ui/input';
import { ListRow } from '@/components/ui/list-row';
import { PersonActionsMenu } from '@/components/People/PersonActionsMenu';
import { PersonFormDialog } from '@/components/People/PersonFormDialog';
import { VoiceprintSamplesDialog } from '@/components/People/VoiceprintSamplesDialog';
import { ForgetPersonDialog } from './ForgetPersonDialog';
import type { Person } from '@/types';
import { filterPeople } from '@/lib/people-filter';
import { avatarColorClass } from '@/lib/avatar-colors';
import { PageHeader } from '@/components/ui/page-header';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';

/**
 * A single person row: avatar, name + role/email, the voiceprint opt-out switch,
 * and an overflow menu (edit / forget). The switch is wired straight to
 * `api_set_person_voiceprint_opt_out` with optimistic local state.
 */
function PersonRow({
  person,
  dotClass,
  onOpen,
  onEdit,
  onManageVoice,
  onRequestForget,
  onOptOutChanged,
  onStarredChanged,
}: {
  person: Person;
  dotClass: string;
  onOpen: (id: string) => void;
  onEdit: (person: Person) => void;
  onManageVoice: (person: Person) => void;
  onRequestForget: (person: Person) => void;
  onOptOutChanged: (id: string, optOut: boolean) => void;
  onStarredChanged: (id: string, starred: boolean) => void;
}) {
  const [saving, setSaving] = useState(false);
  const [starSaving, setStarSaving] = useState(false);
  const name = person.displayName?.trim() || 'Unnamed person';
  // Role + email rendered as a single muted subline ("Role · email").
  const subParts = [person.role?.trim(), person.email?.trim()].filter(
    (s): s is string => !!s,
  );

  const handleToggle = useCallback(
    async (next: boolean) => {
      if (saving) return;
      setSaving(true);
      // Optimistic; revert on failure.
      onOptOutChanged(person.id, next);
      try {
        await invoke('api_set_person_voiceprint_opt_out', { id: person.id, optOut: next });
      } catch (error) {
        console.error('Failed to update voiceprint setting:', error);
        onOptOutChanged(person.id, !next);
        toast.error('Could not update voice setting.', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setSaving(false);
      }
    },
    [person.id, saving, onOptOutChanged],
  );

  // specs/0038 WS5.a — pin/unpin this person to the top of ranked pick-lists.
  // Optimistic; revert on failure.
  const handleToggleStar = useCallback(async () => {
    if (starSaving) return;
    const next = !person.starred;
    setStarSaving(true);
    onStarredChanged(person.id, next);
    try {
      await invoke('api_set_person_starred', { personId: person.id, starred: next });
    } catch (error) {
      console.error('Failed to update starred setting:', error);
      onStarredChanged(person.id, !next);
      toast.error('Could not update star.', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setStarSaving(false);
    }
  }, [person.id, person.starred, starSaving, onStarredChanged]);

  // specs/0019 WS3.2 — compact single-line row: avatar + name (+ a small "voice not
  // stored" indicator) + subline. The verbose opt-out toggle + voice-samples copy moved
  // into the "…" menu, leaving only the indicator inline.
  // specs/0038 dogfood feedback #3 — clicking the row navigates to the dedicated
  // person-detail page. The star and the overflow menu sit OUTSIDE ListRow's main button,
  // so they never trigger navigation.
  return (
    <ListRow
      onClick={() => onOpen(person.id)}
      lead={<Avatar name={name} photo={person.photoDataUri} size={32} colorClass={dotClass} />}
      title={
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="truncate">{name}</span>
          {person.voiceprintOptOut && (
            <Badge tone="outline" title="Voice not stored" aria-label="Voice not stored">
              NO VOICE
            </Badge>
          )}
        </span>
      }
      meta={subParts.length > 0 ? subParts.join(' · ') : undefined}
      trailing={
        // specs/0038 WS5.a — star pins the person to the top of ranked pick-lists.
        // Starred rows keep the filled star visible; unstarred reveal it on hover.
        <IconButton
          variant="ghost"
          label={person.starred ? 'Unstar person' : 'Star person'}
          aria-pressed={person.starred}
          onClick={() => void handleToggleStar()}
          disabled={starSaving}
          icon={
            <DeckIcon icon={Star} size={14} className={person.starred ? 'fill-current' : undefined} />
          }
          className={
            person.starred
              ? 'text-brand hover:text-brand'
              : 'opacity-0 focus-visible:opacity-100 group-hover/row:opacity-100'
          }
        />
      }
      menuReveal="hover"
      menu={
        <PersonActionsMenu
          voiceprintOptOut={person.voiceprintOptOut}
          onEdit={() => onEdit(person)}
          onManageVoice={() => onManageVoice(person)}
          onToggleVoiceOptOut={() => void handleToggle(!person.voiceprintOptOut)}
          optOutSaving={saving}
          onForget={() => onRequestForget(person)}
        />
      }
    />
  );
}

export default function PeoplePage() {
  const router = useRouter();
  const [people, setPeople] = useState<Person[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Create/edit dialog: `undefined`/null target = create, a person = edit. The
  // boolean drives open state so the dialog can fully unmount between opens.
  const [formOpen, setFormOpen] = useState(false);
  const [personToEdit, setPersonToEdit] = useState<Person | null>(null);
  // Person pending "forget" (drives the confirm dialog); null when closed.
  const [personToForget, setPersonToForget] = useState<Person | null>(null);
  // Person whose voice samples are being managed (drives the samples dialog); null when closed.
  const [personToManageVoice, setPersonToManageVoice] = useState<Person | null>(null);

  // specs/0038 dogfood feedback #3 — clicking a row opens the dedicated person-detail page.
  const handleOpen = useCallback(
    (id: string) => router.push(`/person-details?id=${id}`),
    [router],
  );

  const load = useCallback(async () => {
    try {
      // Ranked (specs/0038 WS5.a): starred first, then by call frequency, then
      // alphabetically — so your top collaborators lead the directory.
      const result = (await invoke('api_list_people_ranked')) as Person[];
      setPeople(Array.isArray(result) ? result : []);
      setError(null);
    } catch (err) {
      console.error('Failed to load people:', err);
      setError('Could not load your people.');
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleAdd = useCallback(() => {
    setPersonToEdit(null);
    setFormOpen(true);
  }, []);

  const handleEdit = useCallback((person: Person) => {
    setPersonToEdit(person);
    setFormOpen(true);
  }, []);

  const handleForgotten = useCallback(async () => {
    const id = personToForget?.id;
    if (id) setPeople((prev) => prev.filter((p) => p.id !== id));
    await load();
  }, [personToForget, load]);

  // Optimistic opt-out flip (reverted by the row on IPC failure).
  const handleOptOutChanged = useCallback((id: string, optOut: boolean) => {
    setPeople((prev) =>
      prev.map((p) => (p.id === id ? { ...p, voiceprintOptOut: optOut } : p)),
    );
  }, []);

  // Optimistic star flip (reverted by the row on IPC failure). Keeps existing
  // list order for now; the new ranking applies on the next load.
  const handleStarredChanged = useCallback((id: string, starred: boolean) => {
    setPeople((prev) => prev.map((p) => (p.id === id ? { ...p, starred } : p)));
  }, []);

  // specs/0019 WS3.3 — search the directory (name/role/email) as it grows.
  const [query, setQuery] = useState('');

  // Backend returns ranked order (starred → frequency → name); keep it, then filter.
  const hasPeople = people.length > 0;
  const filtered = useMemo(() => filterPeople(people, query), [people, query]);
  const noMatches = hasPeople && filtered.length === 0;

  return (
    <div className="flex h-page flex-col bg-background animate-fade-in">
      <PageHeader
        title="People"
        subtitle={
          hasPeople
            ? `${people.length} ${people.length === 1 ? 'person' : 'people'}`
            : 'Remember the people you meet with, across meetings'
        }
        actions={
          hasPeople ? (
            <Button variant="brand" onClick={handleAdd} className="flex-shrink-0">
              Add person
            </Button>
          ) : undefined
        }
      />

      {hasPeople && (
        <div className={cn('flex-shrink-0 pb-3', PAGE_GUTTER)}>
          <div className={cn('relative mx-auto', PAGE_CONTENT_WIDTH)}>
            <DeckIcon
              icon={Search}
              size={16}
              className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search people by name, role, or email…"
              className="pl-9"
              aria-label="Search people"
            />
          </div>
        </div>
      )}

      <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
        {isLoading ? (
          <PageLoading fill="pane" label="Loading people…" />
        ) : error ? (
          <div className={cn('mx-auto', PAGE_CONTENT_WIDTH)}>
            <Callout tone="error">{error}</Callout>
          </div>
        ) : (
          <div className={cn('mx-auto', PAGE_CONTENT_WIDTH)}>
            {hasPeople ? (
              noMatches ? (
                <div className="flex h-40 flex-col items-center justify-center text-center">
                  <p className="text-sm text-muted-foreground">
                    No people match “{query.trim()}”.
                  </p>
                </div>
              ) : (
                <div className="flex flex-col gap-0.5">
                  {filtered.map((person) => (
                    <PersonRow
                      key={person.id}
                      person={person}
                      dotClass={avatarColorClass(person.id)}
                      onOpen={handleOpen}
                      onEdit={handleEdit}
                      onManageVoice={setPersonToManageVoice}
                      onRequestForget={setPersonToForget}
                      onOptOutChanged={handleOptOutChanged}
                      onStarredChanged={handleStarredChanged}
                    />
                  ))}
                </div>
              )
            ) : (
              <EmptyState
                className="h-[40vh]"
                title="No people yet"
                body="Add the people you meet with, then attach them to speakers in your transcripts."
                action={
                  <Button variant="brand" onClick={handleAdd}>
                    Add a person
                  </Button>
                }
              />
            )}
          </div>
        )}
      </div>

      <PersonFormDialog
        open={formOpen}
        onOpenChange={setFormOpen}
        person={personToEdit}
        onSaved={load}
      />

      {personToManageVoice && (
        <VoiceprintSamplesDialog
          open={!!personToManageVoice}
          onOpenChange={(open) => {
            if (!open) setPersonToManageVoice(null);
          }}
          personId={personToManageVoice.id}
          personName={personToManageVoice.displayName}
        />
      )}

      {personToForget && (
        <ForgetPersonDialog
          open={!!personToForget}
          onOpenChange={(open) => {
            if (!open) setPersonToForget(null);
          }}
          personId={personToForget.id}
          personName={personToForget.displayName}
          onForgotten={handleForgotten}
        />
      )}
    </div>
  );
}
