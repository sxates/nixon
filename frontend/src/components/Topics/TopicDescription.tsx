'use client';

/**
 * The topic's Description (specs/0089 §UI): collapsed under the header, a textarea that
 * saves on blur. Draft from meetings fills the textarea with an LLM draft but saves
 * nothing until the owner blurs the field or presses Save.
 */

import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { ChevronDown, ChevronRight, Sparkles } from '@/components/ui/icons';
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

  // Take a newer saved description unless the owner has unsaved text in the field.
  useEffect(() => {
    setDraft((cur) => (cur === saved.current ? description : cur));
    saved.current = description;
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
      <div>
        <Button
          size="xs"
          variant="ghost"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
          className="-ml-2.5"
        >
          <DeckIcon icon={open ? ChevronDown : ChevronRight} size={14} />
          Description
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
        <p className="line-clamp-2 text-meta text-muted-foreground">{description}</p>
      ) : null}
    </div>
  );
}
