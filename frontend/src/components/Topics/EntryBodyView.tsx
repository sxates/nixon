'use client';

/**
 * The read-only body of a timeline entry (specs/0089): bullets with their time links, then
 * Decision call-outs (`Callout tone="success"`) with the revisions attached to them, dated. A revision
 * whose decision is no longer in the body (edited out, or re-minted by a rewrite) renders
 * at the entry's foot so it is never lost.
 */

import { Badge } from '@/components/ui/badge';
import { Callout } from '@/components/ui/callout';
import { formatDecisionDate } from '@/lib/topics';
import { cn } from '@/lib/utils';
import type { EntryBody, TopicRevision } from '@/types';
import { MomentLink } from './MomentLink';

export interface EntryBodyViewProps {
  body: EntryBody;
  /** The meeting the times point into; null hides time links (manual, or source deleted). */
  meetingId: string | null;
  revisions?: TopicRevision[];
  className?: string;
}

export function EntryBodyView({ body, meetingId, revisions = [], className }: EntryBodyViewProps) {
  const decisionIds = new Set(body.decisions.map((d) => d.id));
  const orphans = revisions.filter((r) => !r.decisionId || !decisionIds.has(r.decisionId));

  return (
    <div className={cn('flex flex-col gap-2', className)}>
      {body.bullets.length > 0 ? (
        <ul className="flex list-disc flex-col gap-1 pl-5 text-sm text-foreground marker:text-muted-foreground">
          {body.bullets.map((b, i) => (
            <li key={i}>
              {b.text}
              {meetingId && b.t != null ? (
                <>
                  {' '}
                  <MomentLink meetingId={meetingId} t={b.t} />
                </>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      {body.decisions.map((d) => (
        <Callout
          key={d.id || d.text}
          tone="success"
          size="compact"
          data-decision-id={d.id}
          className="flex flex-col gap-1"
        >
          <div className="flex items-baseline gap-2">
            <Badge tone="success">Decision</Badge>
            {meetingId && d.t != null ? <MomentLink meetingId={meetingId} t={d.t} /> : null}
          </div>
          <p className="text-sm text-foreground">{d.text}</p>
          {revisions
            .filter((r) => r.decisionId === d.id)
            .map((r) => (
              <RevisionLine key={r.id} r={r} />
            ))}
        </Callout>
      ))}
      {orphans.length > 0 ? (
        <div data-entry-foot className="flex flex-col gap-1 border-t border-rule pt-2">
          {orphans.map((r) => (
            <RevisionLine key={r.id} r={r} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function RevisionLine({ r }: { r: TopicRevision }) {
  const when = formatDecisionDate(r.createdAt);
  return (
    <p className="text-meta text-muted-foreground">
      <span className="font-semibold text-foreground">{when ? `${when}:` : 'Later:'}</span> {r.text}
      {r.sourceDeletedAt ? (
        <span className="text-caption"> · Source meeting deleted</span>
      ) : r.sourceMeetingId && r.tStart != null ? (
        <>
          {' '}
          <MomentLink meetingId={r.sourceMeetingId} t={r.tStart} />
        </>
      ) : null}
    </p>
  );
}
