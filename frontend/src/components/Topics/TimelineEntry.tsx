'use client';

/**
 * One timeline entry (specs/0089 §UI). Machines append, owners edit: the AI's original
 * (`aiBody`) is never changed by an edit, so an edited entry shows an "edited" marker that
 * reveals the original read-only, with Revert to original. Rewrite from meeting re-runs the
 * writer, only while the entry is unedited and still has its meeting.
 */

import { useState } from 'react';
import { toast } from 'sonner';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { DeckIcon } from '@/components/ui/deck-icon';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Sparkles, Trash2 } from '@/components/ui/icons';
import { MenuButton } from '@/components/ui/menu-button';
import { Spinner } from '@/components/ui/spinner';
import { formatMeetingDate } from '@/lib/format-date';
import { deleteTopicEntry, revertTopicEntry, rewriteTopicEntry } from '@/lib/topics';
import type { TopicEntry, TopicEntryKind } from '@/types';
import { EntryBodyView } from './EntryBodyView';
import { EntryEditor } from './EntryEditor';
import { MeetingLink } from './MomentLink';

const KIND_LABEL: Record<TopicEntryKind, string> = {
  series: 'Series',
  mention: 'Mention',
  manual: 'Yours',
};

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface TimelineEntryProps {
  entry: TopicEntry;
  /** Called after a write (the `topics-updated` event also refreshes). */
  onChanged?: () => void;
}

export function TimelineEntry({ entry, onChanged }: TimelineEntryProps) {
  const [editing, setEditing] = useState(false);
  const [showOriginal, setShowOriginal] = useState(false);
  const [busy, setBusy] = useState<'revert' | 'rewrite' | 'delete' | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const meetingId = entry.sourceDeletedAt ? null : entry.meetingId;
  const edited = !!entry.editedAt;
  const empty = entry.body.bullets.length === 0 && entry.body.decisions.length === 0;
  const noDiscussion = empty && !edited && entry.revisions.length === 0;
  const canRewrite = !edited && !!meetingId && !!entry.aiBody;

  const run = async (kind: 'revert' | 'rewrite' | 'delete', fn: () => Promise<void>, fail: string) => {
    setBusy(kind);
    try {
      await fn();
      onChanged?.();
      if (kind === 'revert') setShowOriginal(false);
      if (kind === 'delete') setConfirmDelete(false);
    } catch (e) {
      console.error(`${fail}:`, e);
      toast.error(fail, { description: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const source = entry.sourceDeletedAt ? (
    <span className="text-meta text-muted-foreground">Source meeting deleted</span>
  ) : meetingId ? (
    <MeetingLink meetingId={meetingId} title={entry.meetingTitle?.trim() || 'Untitled meeting'} />
  ) : null;

  const menu = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <MenuButton label="Entry options" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => setConfirmDelete(true)}>
          Delete entry
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  const deleteDialog = (
    <ConfirmDialog
      open={confirmDelete}
      onOpenChange={setConfirmDelete}
      title="Delete this entry?"
      body="It comes off the timeline. The meeting itself isn't touched."
      confirmLabel="Delete entry"
      busy={busy === 'delete'}
      onConfirm={() => run('delete', () => deleteTopicEntry(entry.id), 'Could not delete the entry')}
    />
  );

  const date = <span className="u-row-meta tabular-nums">{formatMeetingDate(entry.occurredAt)}</span>;

  if (noDiscussion) {
    return (
      <article className="flex min-h-7 items-center gap-2" aria-label="Timeline entry">
        {date}
        {source}
        <span className="min-w-0 flex-1 truncate text-meta text-muted-foreground">
          No discussion of this topic
        </span>
        {menu}
        {deleteDialog}
      </article>
    );
  }

  return (
    <article className="flex flex-col gap-2" aria-label="Timeline entry">
      <header className="flex min-h-7 flex-wrap items-center gap-2">
        {date}
        {source}
        <Badge tone={entry.kind === 'manual' ? 'brand' : 'muted'}>{KIND_LABEL[entry.kind]}</Badge>
        {edited ? (
          <Button
            type="button"
            variant="link"
            aria-expanded={showOriginal}
            onClick={() => setShowOriginal((v) => !v)}
            className="h-auto p-0 text-caption font-normal text-muted-foreground"
          >
            edited
          </Button>
        ) : null}
        <div className="ml-auto flex items-center gap-1">
          {!editing ? (
            <Button size="xs" variant="ghost" onClick={() => setEditing(true)}>
              Edit
            </Button>
          ) : null}
          {canRewrite && !editing ? (
            <Button
              size="xs"
              variant="ghost"
              disabled={busy === 'rewrite'}
              onClick={() =>
                void run('rewrite', () => rewriteTopicEntry(entry.id), 'Could not rewrite the entry')
              }
            >
              {busy === 'rewrite' ? <Spinner /> : <DeckIcon icon={Sparkles} size={14} />}
              Rewrite from meeting
            </Button>
          ) : null}
          {menu}
        </div>
      </header>

      {editing ? (
        <EntryEditor
          topicId={entry.topicId}
          entry={entry}
          onDone={() => {
            setEditing(false);
            onChanged?.();
          }}
        />
      ) : (
        <EntryBodyView body={entry.body} meetingId={meetingId} revisions={entry.revisions} />
      )}

      {edited && showOriginal && entry.aiBody ? (
        <div className="flex flex-col gap-2 rounded-md border border-rule bg-muted px-3 py-2">
          <span className="u-section-label">Original</span>
          <EntryBodyView body={entry.aiBody} meetingId={meetingId} />
          <div>
            <Button
              size="xs"
              variant="outline"
              disabled={busy === 'revert'}
              onClick={() =>
                void run('revert', () => revertTopicEntry(entry.id), 'Could not revert the entry')
              }
            >
              Revert to original
            </Button>
          </div>
        </div>
      ) : null}
      {deleteDialog}
    </article>
  );
}
