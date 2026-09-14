'use client';

/**
 * ActionItemsSection (specs/0034) — the per-meeting action items, rendered on the
 * Summary tab below the summary content (items derive from the summary, so they live
 * with it). Items load via `api_get_action_items` and refresh on the
 * `action-items-updated` Tauri event (emitted after a background extraction commits).
 *
 * Rows come from the shared `ActionItemRow` (checkbox / inline edit / assignee chip /
 * due hint / dismiss-or-delete); mutations come from the shared
 * `useActionItemMutations` hook (optimistic updates with per-item error reverts).
 * Assignee candidates are this meeting's participant roster (specs/0017); names for
 * already-assigned people resolve through the People directory so assignees survive
 * roster removal. Dismissed items are hidden here — dismissal is persistent (it
 * protects the row from re-extraction), and the task hub's Dismissed filter is where
 * they can be restored.
 *
 * Empty states (spec): no summary yet → explanatory line only; summary but zero items
 * → "None found" + add row + "Scan again" (`api_extract_action_items` — also the
 * retroactive path for meetings that predate this feature).
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ListTodo, Loader2, ScanSearch } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ActionItemRow } from '@/components/ActionItems/ActionItemRow';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import type { AssigneeCandidate } from '@/components/ActionItems/AssigneePicker';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { assigneeLabel } from '@/lib/action-items';
import { safeListen } from '@/lib/safe-listen';
import type {
  ActionItem,
  ActionItemsUpdatedPayload,
  MeetingParticipant,
  Person,
} from '@/types';

interface ActionItemsSectionProps {
  meetingId: string;
  /** Whether this meeting has a generated summary (drives the empty states). */
  hasSummary: boolean;
}

export function ActionItemsSection({ meetingId, hasSummary }: ActionItemsSectionProps) {
  const [items, setItems] = useState<ActionItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [participants, setParticipants] = useState<MeetingParticipant[]>([]);
  const [people, setPeople] = useState<Person[]>([]);
  const [isScanning, setIsScanning] = useState(false);

  const loadItems = useCallback(async () => {
    try {
      const result = await invoke<ActionItem[]>('api_get_action_items', { meetingId });
      setItems(Array.isArray(result) ? result : []);
    } catch (err) {
      // Best-effort: never crash the summary tab over this section.
      console.error('Failed to load action items:', err);
    } finally {
      setIsLoading(false);
    }
  }, [meetingId]);

  useEffect(() => {
    setIsLoading(true);
    void loadItems();
  }, [loadItems]);

  // Roster (assignee candidates) + People directory (name resolution) — best-effort.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const roster = await invoke<MeetingParticipant[]>('api_get_meeting_participants', {
          meetingId,
        });
        if (!cancelled) setParticipants(Array.isArray(roster) ? roster : []);
      } catch (err) {
        console.error('Failed to load participants for action items:', err);
      }
      try {
        const directory = await invoke<Person[]>('api_list_people');
        if (!cancelled) setPeople(Array.isArray(directory) ? directory : []);
      } catch (err) {
        console.error('Failed to load people for action items:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  // Refresh when a background extraction commits for THIS meeting.
  useEffect(
    () =>
      safeListen<ActionItemsUpdatedPayload>('action-items-updated', (event) => {
        if (event.payload?.meeting_id === meetingId) void loadItems();
      }),
    [meetingId, loadItems],
  );

  const nameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const person of people) map.set(person.id, person.displayName);
    for (const participant of participants)
      map.set(participant.personId, participant.displayName);
    return map;
  }, [people, participants]);

  const candidates = useMemo<AssigneeCandidate[]>(
    () =>
      participants.map((p) => ({
        personId: p.personId,
        displayName: p.displayName,
        email: p.email,
      })),
    [participants],
  );

  const { setStatus, editDescription, assign, deleteItem, add } = useActionItemMutations({
    setItems,
    meetingId,
    onStatusChanged: (updated) => {
      if (updated.status === 'dismissed') {
        toast.success('Item dismissed', {
          description: "It won't come back on the next scan.",
        });
      }
    },
  });

  const handleScan = useCallback(async () => {
    setIsScanning(true);
    try {
      // Returns the number of rows the run inserted or updated — 0 on a no-op
      // (unchanged summary), an in-flight skip, or when every candidate was absorbed
      // by an existing dismissed/completed row. The list itself refreshes via the
      // `action-items-updated` event after the run commits (no explicit reload here).
      const count = await invoke<number>('api_extract_action_items', { meetingId });
      toast.success(
        count === 0
          ? 'No new action items'
          : `${count} action ${count === 1 ? 'item' : 'items'} added or updated`,
      );
    } catch (err) {
      console.error('Failed to extract action items:', err);
      toast.error('Could not scan for action items', {
        description: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsScanning(false);
    }
  }, [meetingId]);

  // Dismissed items are hidden on the meeting page (restorable from the hub).
  const visible = useMemo(() => items.filter((i) => i.status !== 'dismissed'), [items]);

  const scanButton = (
    <Button
      type="button"
      size="sm"
      variant="outline"
      onClick={() => void handleScan()}
      disabled={isScanning}
      className="h-8 gap-1.5"
    >
      {isScanning ? (
        <Loader2 size={13} className="animate-spin" />
      ) : (
        <ScanSearch size={13} />
      )}
      {isScanning ? 'Scanning…' : 'Scan again'}
    </Button>
  );

  return (
    <section
      aria-label="Action items"
      className="flex flex-col gap-2 rounded-[3px] border border-border bg-card px-4 py-3"
    >
      <div className="flex items-center gap-2">
        <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <ListTodo size={13} />
          Action items
          {visible.length > 0 && (
            <span className="text-muted-foreground">({visible.length})</span>
          )}
        </span>
      </div>

      {isLoading ? (
        <p className="text-xs text-muted-foreground">Loading…</p>
      ) : visible.length === 0 ? (
        !hasSummary ? (
          <p className="text-xs text-muted-foreground">
            Action items appear after the summary is generated.
          </p>
        ) : (
          <div className="flex flex-col gap-2.5">
            <div className="flex items-center gap-2.5">
              <p className="text-xs text-muted-foreground">
                None found in this meeting&apos;s summary.
              </p>
              {scanButton}
            </div>
            <AddActionItemRow onAdd={add} />
          </div>
        )
      ) : (
        <div className="flex flex-col gap-1">
          <div className="-mx-2 flex flex-col gap-0.5">
            {visible.map((item) => (
              <ActionItemRow
                key={item.id}
                item={item}
                assigneeName={assigneeLabel(item, nameById)}
                candidates={candidates}
                onSetStatus={setStatus}
                onEditDescription={editDescription}
                onAssign={assign}
                onDelete={deleteItem}
              />
            ))}
          </div>
          <AddActionItemRow onAdd={add} className="mt-1" />
        </div>
      )}
    </section>
  );
}
