'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Check, Loader2, MicOff, MoreHorizontal, Pencil, Search, Star, Trash2, UserPlus, Users, Waves } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { PersonAvatar } from '@/components/People/PersonAvatar';
import { PersonFormDialog } from '@/components/People/PersonFormDialog';
import { VoiceprintSamplesDialog } from '@/components/People/VoiceprintSamplesDialog';
import { ForgetPersonDialog } from './ForgetPersonDialog';
import type { Person } from '@/types';
import { filterPeople } from '@/lib/people-filter';
import { avatarColorClass } from '@/lib/avatar-colors';
import { PageHeader } from '@/components/ui/page-header';

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
  // person-detail page. Nested controls (star / overflow) stop propagation so they
  // never trigger navigation.
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen(person.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen(person.id);
        }
      }}
      className="group flex w-full cursor-pointer items-center gap-3 rounded-[3px] px-3 py-2 text-left transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <PersonAvatar
        name={name}
        photoDataUri={person.photoDataUri}
        size="sm"
        colorClass={dotClass}
      />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[13.5px] font-semibold text-foreground">{name}</span>
          {person.voiceprintOptOut && (
            <span
              title="Voice not stored"
              aria-label="Voice not stored"
              className="u-section-label flex-shrink-0 rounded-[2px] border border-border px-1 text-[9px] text-muted-foreground"
            >
              NO VOICE
            </span>
          )}
        </div>
        {subParts.length > 0 && (
          <div className="truncate text-xs text-muted-foreground">{subParts.join(' · ')}</div>
        )}
      </div>

      {/* specs/0038 WS5.a — star pins the person to the top of ranked pick-lists.
          Starred rows keep the filled star visible; unstarred reveal it on hover. */}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          void handleToggleStar();
        }}
        disabled={starSaving}
        aria-label={person.starred ? 'Unstar person' : 'Star person'}
        aria-pressed={person.starred}
        title={person.starred ? 'Starred — pinned to the top' : 'Star to pin to the top'}
        className={`inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md transition-all focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 disabled:opacity-50 ${
          person.starred
            ? 'text-brand hover:bg-background'
            : 'text-muted-foreground opacity-0 hover:bg-background hover:text-foreground group-hover:opacity-100'
        }`}
      >
        <Star className={`h-4 w-4 ${person.starred ? 'fill-current' : ''}`} />
      </button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Person options"
            title="Person options"
            onClick={(e) => e.stopPropagation()}
            className="inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-background hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
          >
            <MoreHorizontal className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => onEdit(person)}>
            <Pencil className="mr-2 h-4 w-4" />
            Edit
          </DropdownMenuItem>
          {/* specs/0039 WS3 — per-sample voice gallery management (quarantine/restore/delete).
              Hidden when this person's voice isn't stored (opt-out ⇒ no samples). */}
          {!person.voiceprintOptOut && (
            <DropdownMenuItem onSelect={() => onManageVoice(person)}>
              <Waves className="mr-2 h-4 w-4" />
              Manage voice samples
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          {/* Voiceprint opt-out — identity/naming still work; this only blocks voice
              modeling. A check marks the active ("don't store") state. */}
          <DropdownMenuItem
            disabled={saving}
            onSelect={(e) => {
              e.preventDefault();
              void handleToggle(!person.voiceprintOptOut);
            }}
          >
            <MicOff className="mr-2 h-4 w-4" />
            Don&apos;t store this person&apos;s voice
            {person.voiceprintOptOut && <Check className="ml-auto h-4 w-4" />}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onSelect={() => onRequestForget(person)}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 className="mr-2 h-4 w-4" />
            Forget this person
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
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
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      <PageHeader
        title="People"
        subtitle={
          hasPeople
            ? `${people.length} ${people.length === 1 ? 'person' : 'people'}`
            : 'Remember the people you meet with, across meetings'
        }
        actions={
          hasPeople ? (
            <Button variant="brand" onClick={handleAdd} className="flex-shrink-0 gap-2">
              <UserPlus className="h-4 w-4" />
              Add person
            </Button>
          ) : undefined
        }
      />

      {hasPeople && (
        <div className="flex-shrink-0 px-4 min-[900px]:px-7 pb-3">
          <div className="relative mx-auto max-w-[840px]">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
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

      <div className="flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12">
        {isLoading ? (
          <div className="flex h-64 items-center justify-center text-muted-foreground">
            <Loader2 className="mr-2 h-5 w-5 animate-spin" />
            <span className="text-sm">Loading people…</span>
          </div>
        ) : error ? (
          <div className="flex h-64 flex-col items-center justify-center text-center">
            <p className="text-sm text-muted-foreground">{error}</p>
          </div>
        ) : (
          <div className="mx-auto max-w-[840px]">
            {hasPeople ? (
              noMatches ? (
                <div className="flex h-40 flex-col items-center justify-center text-center">
                  <p className="text-sm text-muted-foreground">
                    No people match “{query.trim()}”.
                  </p>
                </div>
              ) : (
                <div className="space-y-0.5">
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
              <div className="flex h-[40vh] flex-col items-center justify-center text-center">
                <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-muted">
                  <Users className="h-6 w-6 text-muted-foreground" />
                </div>
                <h2 className="text-base font-semibold text-foreground">No people yet</h2>
                <p className="mt-1 max-w-xs text-sm text-muted-foreground">
                  Add the people you meet with, then attach them to speakers in your transcripts.
                </p>
                <Button variant="brand" onClick={handleAdd} className="mt-5 gap-2">
                  <UserPlus className="h-4 w-4" />
                  Add a person
                </Button>
              </div>
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
    </motion.div>
  );
}
