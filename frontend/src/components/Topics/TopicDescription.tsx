'use client';

/**
 * The topic's Description (specs/0089 §UI): shown in full under the header (it's how a topic
 * is recognised), with Edit opening a textarea that saves on blur. Draft from meetings fills
 * the textarea with an LLM draft but saves nothing until the owner blurs the field or
 * presses Save.
 */

import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { Sparkles } from '@/components/ui/icons';
import { Spinner } from '@/components/ui/spinner';
import { Textarea } from '@/components/ui/textarea';
import { draftTopicDescription, updateTopic } from '@/lib/topics';

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface TopicDescriptionProps {
  topicId: string;
  description: string;
  onChanged?: () => void;
}

export function TopicDescription({ topicId, description, onChanged }: TopicDescriptionProps) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(description);
  const [drafting, setDrafting] = useState(false);
  const saved = useRef(description);

  // Take a newer saved description unless the owner has unsaved text in the field. Compare
  // against the previous saved text, captured before it is overwritten (the updater can run
  // after this effect returns).
  useEffect(() => {
    const prev = saved.current;
    saved.current = description;
    setDraft((cur) => (cur === prev ? description : cur));
  }, [description]);

  const save = async () => {
    const text = draft.trim();
    if (text === saved.current.trim()) return;
    try {
      await updateTopic(topicId, { description: text });
      saved.current = text;
      setDraft(text);
      onChanged?.();
    } catch (e) {
      console.error('Failed to save the description:', e);
      toast.error('Could not save the description', { description: errorText(e) });
    }
  };

  const draftFromMeetings = async () => {
    setDrafting(true);
    try {
      setDraft(await draftTopicDescription(topicId));
    } catch (e) {
      console.error('Failed to draft the description:', e);
      toast.error('Could not draft a description', { description: errorText(e) });
    } finally {
      setDrafting(false);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <span className="u-section-label-sm">Description</span>
        <Button
          size="xs"
          variant="ghost"
          aria-label={open ? 'Done editing the description' : 'Edit description'}
          onClick={() => setOpen((v) => !v)}
        >
          {open ? 'Done' : 'Edit'}
        </Button>
      </div>
      {open ? (
        <>
          <Textarea
            aria-label="Description"
            value={draft}
            rows={3}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => void save()}
            placeholder="What this topic covers"
          />
          <div className="flex items-center gap-2">
            <Button size="xs" variant="outline" disabled={drafting} onClick={() => void draftFromMeetings()}>
              {drafting ? <Spinner /> : <DeckIcon icon={Sparkles} size={14} />}
              Draft from meetings
            </Button>
            <Button
              size="xs"
              variant="ghost"
              disabled={draft.trim() === saved.current.trim()}
              onClick={() => void save()}
            >
              Save
            </Button>
          </div>
        </>
      ) : description.trim() ? (
        <p className="whitespace-pre-line text-meta text-muted-foreground">{description}</p>
      ) : null}
    </div>
  );
}
