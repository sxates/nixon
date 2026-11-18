'use client';

/**
 * The topic page header (specs/0089 §UI). `TopicHeader` is the pinned title row: back, the
 * click-to-rename name, a star (lists the topic in the sidebar while open; specs/0090), and
 * an overflow menu with Archive / Unarchive topic and Delete topic. (Whether its to-dos show
 * on the To-do list is the "Show in To-Do" switch on its action items card.)
 * `TopicMeta` sits under it and scrolls with the page: the Description. `TopicSeriesLinks`
 * (the link chips, Link a Series…, the Meeting count) sits further down, above the timeline.
 *
 * Archived is the user-facing name of the stored `closed` status.
 */

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { DeckIcon } from '@/components/ui/deck-icon';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { IconButton } from '@/components/ui/icon-button';
import { Archive, ArchiveRestore, Star, Trash2 } from '@/components/ui/icons';
import { MenuButton } from '@/components/ui/menu-button';
import { PageHeader } from '@/components/ui/page-header';
import { deleteTopic, updateTopic } from '@/lib/topics';
import { useBack } from '@/hooks/useBack';
import type { TopicDetail } from '@/types';
import { TopicDescription } from './TopicDescription';
import { TopicLinks } from './TopicLinks';

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface TopicHeaderProps {
  detail: TopicDetail;
  onChanged?: () => void;
}

export function TopicHeader({ detail, onChanged }: TopicHeaderProps) {
  const router = useRouter();
  const back = useBack('/topics');
  const { topic } = detail;
  const archived = topic.status === 'closed';
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(topic.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // Optimistic: the star flips at once; the refetch settles it.
  const [starred, setStarred] = useState(topic.starred);
  useEffect(() => setStarred(topic.starred), [topic.starred]);

  const write = async (patch: Parameters<typeof updateTopic>[1], fail: string) => {
    try {
      await updateTopic(topic.id, patch);
      onChanged?.();
      return true;
    } catch (e) {
      console.error(`${fail}:`, e);
      toast.error(fail, { description: errorText(e) });
      return false;
    }
  };

  const toggleStar = async () => {
    const next = !starred;
    setStarred(next);
    const fail = next ? 'Could not star the Topic' : 'Could not unstar the Topic';
    if (!(await write({ starred: next }, fail))) setStarred(!next);
  };

  const commitName = () => {
    setEditingName(false);
    const name = nameDraft.trim();
    if (!name || name === topic.name) {
      setNameDraft(topic.name);
      return;
    }
    void write({ name }, 'Could not rename the Topic');
  };

  const remove = async () => {
    setDeleting(true);
    try {
      await deleteTopic(topic.id);
      setConfirmDelete(false);
      router.push('/topics');
    } catch (e) {
      console.error('Failed to delete topic:', e);
      toast.error('Could not delete the Topic', { description: errorText(e) });
      setDeleting(false);
    }
  };

  const actions = (
    <div className="flex items-center gap-1">
      <IconButton
        variant="ghost"
        label={starred ? 'Unstar Topic' : 'Star Topic'}
        aria-pressed={starred}
        onClick={() => void toggleStar()}
        // The gold goes on the glyph, not the button: PageHeader's actions slot recolours
        // every ghost button (`[&_[data-variant=ghost]]:text-foreground/80`), which beats a
        // class on the button itself.
        icon={<DeckIcon icon={Star} size={14} className={starred ? 'fill-current text-star' : undefined} />}
      />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <MenuButton label="Topic options" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {archived ? (
            <DropdownMenuItem
              icon={ArchiveRestore}
              onSelect={() => void write({ status: 'open' }, 'Could not unarchive the Topic')}
            >
              Unarchive Topic
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem
              icon={Archive}
              onSelect={() => void write({ status: 'closed' }, 'Could not archive the Topic')}
            >
              Archive Topic
            </DropdownMenuItem>
          )}
          <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => setConfirmDelete(true)}>
            Delete Topic
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );

  return (
    <>
      <PageHeader
        width="page"
        back={{ label: 'Back', onClick: back }}
        title={
          <MeetingTitleField
            title={topic.name}
            placeholder="Untitled Topic"
            ariaLabel="Topic name"
            isEditing={editingName}
            draft={nameDraft}
            onDraftChange={setNameDraft}
            onStartEditing={() => {
              setNameDraft(topic.name);
              setEditingName(true);
            }}
            onCommit={commitName}
            onCancel={() => {
              setNameDraft(topic.name);
              setEditingName(false);
            }}
          />
        }
        subtitle={archived ? 'Archived' : undefined}
        actions={actions}
      />
      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title="Delete this Topic?"
        body="Its timeline, your notes, and its suggestions go with it. Meetings and To-Dos stay."
        confirmLabel="Delete Topic"
        busy={deleting}
        onConfirm={remove}
      />
    </>
  );
}

/** The Description, under the pinned title row (scrolls with the body). */
export function TopicMeta({ detail, onChanged }: TopicHeaderProps) {
  const { topic } = detail;
  return <TopicDescription topicId={topic.id} description={topic.description} onChanged={onChanged} />;
}

/** The linked Series chips, Link a Series…, and the Meeting count: sits directly above the
 *  timeline those Series feed (v0.18 feedback). */
export function TopicSeriesLinks({ detail, onChanged }: TopicHeaderProps) {
  const { topic, links, entries } = detail;
  const meetingCount = new Set(entries.map((e) => e.meetingId).filter(Boolean)).size;
  return <TopicLinks topicId={topic.id} links={links} meetingCount={meetingCount} onChanged={onChanged} />;
}
