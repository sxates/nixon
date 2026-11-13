'use client';

/**
 * The topic page header (specs/0089 §UI): the click-to-rename name, the Open / Closed
 * toggle and an overflow menu with Delete topic in the title row; under it the link chips,
 * Link a series…, the meeting count, and the collapsible Description.
 */

import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { MeetingTitleField } from '@/components/MeetingDetails/MeetingTitleField';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Trash2 } from '@/components/ui/icons';
import { MenuButton } from '@/components/ui/menu-button';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl } from '@/components/ui/segmented-control';
import { deleteTopic, updateTopic } from '@/lib/topics';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { TopicDetail, TopicStatus } from '@/types';
import { TopicDescription } from './TopicDescription';
import { TopicLinks } from './TopicLinks';

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface TopicHeaderProps {
  detail: TopicDetail;
  onChanged?: () => void;
}

export function TopicHeader({ detail, onChanged }: TopicHeaderProps) {
  const router = useRouter();
  const { topic, links, entries } = detail;
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(topic.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const meetingCount = new Set(entries.map((e) => e.meetingId).filter(Boolean)).size;

  const write = async (patch: Parameters<typeof updateTopic>[1], fail: string) => {
    try {
      await updateTopic(topic.id, patch);
      onChanged?.();
    } catch (e) {
      console.error(`${fail}:`, e);
      toast.error(fail, { description: errorText(e) });
    }
  };

  const commitName = () => {
    setEditingName(false);
    const name = nameDraft.trim();
    if (!name || name === topic.name) {
      setNameDraft(topic.name);
      return;
    }
    void write({ name }, 'Could not rename the topic');
  };

  const remove = async () => {
    setDeleting(true);
    try {
      await deleteTopic(topic.id);
      setConfirmDelete(false);
      router.push('/topics');
    } catch (e) {
      console.error('Failed to delete topic:', e);
      toast.error('Could not delete the topic', { description: errorText(e) });
      setDeleting(false);
    }
  };

  const actions = (
    <div className="flex items-center gap-2">
      <SegmentedControl
        aria-label="Topic status"
        value={topic.status}
        onChange={(v) => void write({ status: v as TopicStatus }, 'Could not change the status')}
        options={[
          { value: 'open', label: 'Open' },
          { value: 'closed', label: 'Closed' },
        ]}
      />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <MenuButton label="Topic options" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => setConfirmDelete(true)}>
            Delete topic
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );

  return (
    <>
      <PageHeader
        width="page"
        back={{ label: 'Topics', onClick: () => router.push('/topics') }}
        title={
          <MeetingTitleField
            title={topic.name}
            placeholder="Untitled topic"
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
        actions={actions}
      />
      <div className={cn('mx-auto flex w-full flex-col gap-2', PAGE_CONTENT_WIDTH, PAGE_GUTTER)}>
        <TopicLinks topicId={topic.id} links={links} meetingCount={meetingCount} onChanged={onChanged} />
        <TopicDescription topicId={topic.id} description={topic.description} onChanged={onChanged} />
      </div>
      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title="Delete this topic?"
        body="Its timeline, your notes, and its suggestions go with it. Meetings and to-dos stay."
        confirmLabel="Delete topic"
        busy={deleting}
        onConfirm={remove}
      />
    </>
  );
}
