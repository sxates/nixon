'use client';

/**
 * ⌘⇧T quick add (specs/0088): a small dialog over whatever page you are on. Type the to-do
 * (@mentions and a trailing "(Friday)" work as in the To-do page's quick-add), Enter adds
 * it to your list and closes. While a recording runs, "Add to ‹meeting›" (on by default)
 * links the to-do to that meeting. It no longer writes a "☐ To-do: …" line into My Notes:
 * the record screen's To-Dos list shows the item (owner feedback, v0.18).
 */

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { MentionInput, type MentionTopic } from '@/components/ActionItems/MentionInput';
import { TopicPicker } from '@/components/ActionItems/TopicPicker';
import { useTopics } from '@/hooks/useTopics';
import { setActionItemTopic } from '@/lib/topics';
import { keptOffTopic, toastKeptOff } from '@/components/ActionItems/keptOffToast';
import { parseTodoText } from '@/lib/todo-parse';

export interface LiveMeeting {
  id: string;
  title: string;
}

interface QuickAddDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The meeting being recorded right now, if any. */
  liveMeeting: LiveMeeting | null;
}

export function QuickAddDialog({ open, onOpenChange, liveMeeting }: QuickAddDialogProps) {
  const router = useRouter();
  const [draft, setDraft] = useState('');
  const [linkMeeting, setLinkMeeting] = useState(true);
  const [busy, setBusy] = useState(false);
  const [topic, setTopic] = useState<MentionTopic | null>(null);
  const { data: openTopics } = useTopics('open');
  // Each opening starts fresh: empty field, meeting link back on.
  const [resetKey, setResetKey] = useState(0);
  useEffect(() => {
    if (!open) return;
    setDraft('');
    setTopic(null);
    setLinkMeeting(true);
    setResetKey((k) => k + 1);
  }, [open]);

  const submit = async (raw: string = draft) => {
    const text = raw.trim();
    if (!text || busy) return;
    const { title, dueDate } = parseTodoText(text, new Date());
    const description = title || text;
    const meeting = linkMeeting ? liveMeeting : null;
    setBusy(true);
    let tagFailed: string | null = null;
    try {
      const created = await invoke<{ id: string } | null>('api_create_action_item', {
        meetingId: meeting?.id ?? null,
        description,
        dueDate: title ? dueDate : null,
      });
      if (topic && created?.id) {
        try {
          await setActionItemTopic(created.id, topic.id);
        } catch (err) {
          tagFailed = err instanceof Error ? err.message : String(err);
        }
      }
    } catch (err) {
      setBusy(false);
      toast.error('Could not add the To-Do', {
        description: err instanceof Error ? err.message : String(err),
      });
      return; // keep the dialog and the text for a retry
    }
    setBusy(false);
    onOpenChange(false);
    if (tagFailed) {
      toast.error('Added, but not to the Topic', { description: tagFailed });
      return;
    }
    const keptOff = keptOffTopic(openTopics, topic?.id);
    if (keptOff) {
      toastKeptOff('added', keptOff, (id) => router.push(`/topics/topic?id=${encodeURIComponent(id)}`), 'your To-Do list');
      return;
    }
    toast.success(meeting ? `To-Do added to your list and to “${meeting.title}”` : 'To-Do added', {
      action: { label: 'View', onClick: () => router.push('/tasks?tab=todo') },
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>New To-Do</DialogTitle>
          <DialogDescription>
            Enter adds it to your list. @ to mention someone or tag a Topic, a trailing (Friday) sets the due date.
          </DialogDescription>
        </DialogHeader>
        <MentionInput
          key={resetKey}
          value=""
          onChange={setDraft}
          onSubmit={(v) => void submit(v)}
          onCancel={() => onOpenChange(false)}
          placeholder="What needs doing?"
          ariaLabel="Quick add To-Do"
          autoFocus
          className="h-9 text-sm"
          topics={openTopics ?? undefined}
          topicId={topic?.id ?? null}
          onTopicChange={setTopic}
        />
        {liveMeeting && (
          <Checkbox
            checked={linkMeeting}
            onCheckedChange={(v) => setLinkMeeting(v === true)}
            label={`Add to “${liveMeeting.title}”`}
          />
        )}
        <DialogFooter>
          {/* The Topic pick, kept in step with an `@topic` in the text (specs/0090). */}
          <TopicPicker
            topics={openTopics ?? []}
            value={topic}
            onChange={setTopic}
            className="sm:mr-auto"
          />
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" variant="brand" disabled={!draft.trim() || busy} onClick={() => void submit()}>
            Add To-Do
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
