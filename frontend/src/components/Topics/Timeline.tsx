'use client';

/**
 * The topic's append-only timeline (specs/0089 §UI): one entry per meeting, newest first
 * (the backend's order). Its header carries Sparkles, the design-system mark for
 * AI-maintained content. Add entry writes a manual entry by hand.
 */

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { Plus, Sparkles } from '@/components/ui/icons';
import { Panel } from '@/components/ui/panel';
import type { TopicEntry } from '@/types';
import { EntryEditor } from './EntryEditor';
import { TimelineEntry } from './TimelineEntry';

export interface TimelineProps {
  topicId: string;
  entries: TopicEntry[];
  onChanged?: () => void;
}

export function Timeline({ topicId, entries, onChanged }: TimelineProps) {
  const [adding, setAdding] = useState(false);

  return (
    <Panel
      title={
        <span className="inline-flex items-center gap-1.5">
          <DeckIcon icon={Sparkles} size={12} />
          Timeline
        </span>
      }
      action={
        adding ? null : (
          <Button size="xs" variant="ghost" onClick={() => setAdding(true)}>
            <DeckIcon icon={Plus} size={14} />
            Add entry
          </Button>
        )
      }
    >
      <div className="flex flex-col">
        {adding ? (
          <div className="border-b border-rule pb-3">
            <EntryEditor
              topicId={topicId}
              onDone={() => {
                setAdding(false);
                onChanged?.();
              }}
            />
          </div>
        ) : null}
        {entries.length === 0 && !adding ? (
          <p className="py-2 text-sm text-muted-foreground">Nothing logged yet</p>
        ) : (
          entries.map((e) => (
            <div key={e.id} className="border-b border-rule py-3 last:border-b-0 last:pb-1">
              <TimelineEntry entry={e} onChanged={onChanged} />
            </div>
          ))
        )}
      </div>
    </Panel>
  );
}
