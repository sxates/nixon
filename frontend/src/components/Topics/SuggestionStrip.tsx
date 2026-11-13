'use client';

/**
 * Suggested notes (specs/0089 §UI): pending suggestions the mention detector found. Shown
 * only when there is at least one. A `note` row offers Add to topic / Dismiss; a
 * `revision` row names the decision it may change and offers Attach note / Dismiss.
 */

import { useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Panel } from '@/components/ui/panel';
import { formatDecisionDate, resolveTopicSuggestion } from '@/lib/topics';
import type { TopicSuggestion } from '@/types';
import { MeetingLink, MomentLink } from './MomentLink';

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface SuggestionStripProps {
  suggestions: TopicSuggestion[];
  /** Called after a suggestion resolves (the `topics-updated` event also refreshes). */
  onChanged?: () => void;
}

export function SuggestionStrip({ suggestions, onChanged }: SuggestionStripProps) {
  const [busy, setBusy] = useState<string | null>(null);
  if (suggestions.length === 0) return null;

  const resolve = async (s: TopicSuggestion, accept: boolean) => {
    setBusy(s.id);
    try {
      await resolveTopicSuggestion(s.id, accept);
      onChanged?.();
    } catch (e) {
      console.error('Failed to resolve topic suggestion:', e);
      toast.error(accept ? 'Could not add that note' : 'Could not dismiss that note', {
        description: errorText(e),
      });
    } finally {
      setBusy(null);
    }
  };

  return (
    <Panel title="Suggested notes" count={suggestions.length}>
      <ul className="flex flex-col">
        {suggestions.map((s) => (
          <li key={s.id} className="flex flex-col gap-1.5 border-b border-rule py-3 last:border-b-0 last:pb-1">
            <SuggestionRow
              s={s}
              busy={busy === s.id}
              onAccept={() => void resolve(s, true)}
              onDismiss={() => void resolve(s, false)}
            />
          </li>
        ))}
      </ul>
    </Panel>
  );
}

function SuggestionRow({
  s,
  busy,
  onAccept,
  onDismiss,
}: {
  s: TopicSuggestion;
  busy: boolean;
  onAccept: () => void;
  onDismiss: () => void;
}) {
  const revision = s.kind === 'revision';
  const when = s.targetOccurredAt ? formatDecisionDate(s.targetOccurredAt) : '';
  return (
    <>
      <div className="flex min-w-0 items-baseline gap-2">
        <MeetingLink meetingId={s.meetingId} title={s.meetingTitle?.trim() || 'Untitled meeting'} />
        {s.tStart != null ? <MomentLink meetingId={s.meetingId} t={s.tStart} /> : null}
      </div>
      {revision ? (
        <p className="text-sm font-medium text-foreground">
          {`May change the ${when} decision: ${s.targetDecisionText ?? ''}`}
        </p>
      ) : null}
      <p className={revision ? 'text-sm text-muted-foreground' : 'text-sm text-foreground'}>{s.text}</p>
      <div className="flex items-center gap-2 pt-0.5">
        <Button size="xs" variant="outline" disabled={busy} onClick={onAccept}>
          {revision ? 'Attach note' : 'Add to topic'}
        </Button>
        <Button size="xs" variant="ghost" disabled={busy} onClick={onDismiss}>
          Dismiss
        </Button>
      </div>
    </>
  );
}
