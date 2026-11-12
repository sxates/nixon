'use client';

/**
 * InboxRow (specs/0088) — one suggested to-do waiting for triage. Presentation only; the
 * page owns the IPC. Shows the description (person mentions as live names), where it came
 * from (source meeting or "from N meetings" + the latest date), and the context line the
 * extractor quoted. Three outcomes:
 *   - Add to my list — adopt it onto the to-do list;
 *   - Dismiss — decline it (re-extraction won't bring it back);
 *   - Assign… — hand it to someone else; choosing "Me" is the same as Add to my list.
 *
 * `mode="dismissed"` is the "Show dismissed" view: the same row with a single Restore.
 */

import { Button } from '@/components/ui/button';
import { formatMeetingDate } from '@/lib/format-date';
import { titleSegments } from '@/lib/todo-parse';
import type { TodoRow } from '@/types';
import { AssigneePicker, type AssigneeCandidate, type AssigneeSelection } from './AssigneePicker';

interface InboxRowProps {
  item: TodoRow;
  mode?: 'inbox' | 'dismissed';
  nameById: Map<string, string>;
  candidates: AssigneeCandidate[];
  onAdd: (item: TodoRow) => void | Promise<void>;
  onDismiss: (item: TodoRow) => void | Promise<void>;
  onAssign: (item: TodoRow, selection: AssigneeSelection) => void | Promise<void>;
  onRestore: (item: TodoRow) => void | Promise<void>;
  onOpen: (item: TodoRow) => void;
}

function sourceLine(item: TodoRow): string {
  const date = formatMeetingDate(item.latestSourceAt ?? item.meetingCreatedAt);
  const where =
    item.sourceCount > 1
      ? `from ${item.sourceCount} meetings`
      : item.meetingTitle?.trim() || 'Untitled meeting';
  return date ? `${where} · ${date}` : where;
}

export function InboxRow({
  item,
  mode = 'inbox',
  nameById,
  candidates,
  onAdd,
  onDismiss,
  onAssign,
  onRestore,
  onOpen,
}: InboxRowProps) {
  const segments = titleSegments(item.description, nameById);
  return (
    <div className="group flex flex-col gap-1.5 rounded-lg px-2 py-2 transition-colors hover:bg-accent/60">
      <Button
        variant="ghost"
        onClick={() => onOpen(item)}
        title="Open this item"
        className="h-auto min-w-0 justify-start whitespace-normal rounded-md px-0.5 py-0 text-left text-sm font-normal leading-snug text-foreground hover:bg-transparent hover:text-brand"
      >
        {segments.map((seg, i) =>
          seg.kind === 'person' ? (
            <span key={i} className="font-medium">
              @{seg.name}
            </span>
          ) : (
            <span key={i}>{seg.text}</span>
          ),
        )}
      </Button>
      <div className="px-0.5 text-caption text-muted-foreground">{sourceLine(item)}</div>
      {item.latestContext && (
        <p className="line-clamp-2 px-0.5 text-xs text-muted-foreground">{item.latestContext}</p>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        {mode === 'dismissed' ? (
          <Button variant="outline" size="xs" onClick={() => void onRestore(item)}>
            Restore
          </Button>
        ) : (
          <>
            <Button variant="brand" size="xs" onClick={() => void onAdd(item)}>
              Add to my list
            </Button>
            <Button variant="outline" size="xs" onClick={() => void onDismiss(item)}>
              Dismiss
            </Button>
            <AssigneePicker
              label={null}
              emptyLabel="Assign…"
              candidates={candidates}
              onSelect={(selection) => void onAssign(item, selection)}
            />
          </>
        )}
      </div>
    </div>
  );
}
