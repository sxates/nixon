'use client';

/**
 * LinkMeetingPicker (specs/0041 WS4) — the "Link previous meeting…" dialog on the Prep tab.
 *
 * Lists recent recorded meetings (from the existing `api_get_meetings` list command — the
 * same surface the Home dashboard / ⌘K palette use; `scheduled` prep placeholders are
 * already excluded server-side), filterable by title client-side. Picking one calls
 * `api_link_meeting_to_series`, pinning it into the CURRENT meeting's series so prep briefs
 * and carryover can draw from it.
 *
 * Topics (specs/0089) reuse the same search with their own `onPick` (link a series to a
 * topic), `title`, `description`, and extra controls as `children` under the filter.
 * With `multiple`, rows toggle a checkbox and a footer button runs `onPick` for each
 * selected meeting in turn (a topic links several series at once).
 */

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { ListRow } from '@/components/ui/list-row';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { formatMeetingDate } from '@/lib/format-date';

/** The `api_get_meetings` fields the picker needs (serialized camelCase). */
interface PickerMeeting {
  id: string;
  title: string;
  createdAt: string;
}

const MAX_LISTED = 50;

export function LinkMeetingPicker({
  open,
  onOpenChange,
  targetMeetingId,
  excludeIds,
  onLink,
  onPick,
  title = 'Link previous meeting',
  description,
  children,
  multiple = false,
  confirmLabel = (n) => `Link ${n}`,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The meeting whose series the picked meeting is linked into (unused with `onPick`). */
  targetMeetingId?: string;
  /** Meetings to hide from the list (the target itself + already-linked meetings). */
  excludeIds: string[];
  /** Called after a successful link so the parent can reload its prep view. */
  onLink: () => void;
  /** Replaces the series link: what to do with the picked meeting. Rejects to show an error. */
  onPick?: (meetingId: string) => Promise<void>;
  title?: string;
  description?: React.ReactNode;
  /** Extra controls under the filter (a topic's backfill checkbox). */
  children?: React.ReactNode;
  /** Pick several meetings, then confirm with the footer button (requires `onPick`). */
  multiple?: boolean;
  /** The footer button's text for `n` selected meetings (`multiple` only). */
  confirmLabel?: (n: number) => string;
}) {
  const [meetings, setMeetings] = useState<PickerMeeting[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [linkingId, setLinkingId] = useState<string | null>(null);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [linkingMany, setLinkingMany] = useState(false);

  // (Re)load the recent-meetings list each time the picker opens.
  useEffect(() => {
    if (!open) return;
    setFilter('');
    setLinkError(null);
    setSelected([]);
    void (async () => {
      try {
        const result = await invoke<PickerMeeting[]>('api_get_meetings');
        setMeetings(Array.isArray(result) ? result : []);
        setLoadError(null);
      } catch (err) {
        console.error('Failed to load meetings for the link picker:', err);
        setLoadError('We could not load your meetings. Please try again.');
      }
    })();
  }, [open]);

  const excluded = useMemo(() => new Set(excludeIds), [excludeIds]);
  const listed = useMemo(() => {
    if (!meetings) return [];
    const needle = filter.trim().toLowerCase();
    return meetings
      .filter((m) => !excluded.has(m.id))
      .filter((m) => needle === '' || m.title.toLowerCase().includes(needle))
      .slice(0, MAX_LISTED);
  }, [meetings, excluded, filter]);

  const link = async (id: string) => {
    setLinkingId(id);
    setLinkError(null);
    try {
      if (onPick) {
        await onPick(id);
      } else {
        await invoke('api_link_meeting_to_series', {
          meetingId: id,
          targetMeetingId,
        });
      }
      onOpenChange(false);
      onLink();
    } catch (err) {
      console.error('Failed to link meeting to series:', err);
      setLinkError('We could not link that meeting. Please try again.');
    } finally {
      setLinkingId(null);
    }
  };

  const toggle = (id: string) =>
    setSelected((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));

  // Links each selected meeting in turn; the ones that fail stay selected so a retry only
  // repeats those.
  const linkSelected = async () => {
    if (!onPick || selected.length === 0) return;
    setLinkingMany(true);
    setLinkError(null);
    const failed: string[] = [];
    for (const id of selected) {
      try {
        await onPick(id);
      } catch (err) {
        console.error('Failed to link meeting:', err);
        failed.push(id);
      }
    }
    setLinkingMany(false);
    if (failed.length < selected.length) onLink();
    if (failed.length === 0) {
      onOpenChange(false);
    } else {
      setSelected(failed);
      setLinkError(
        failed.length === 1
          ? 'We could not link one of them. Please try again.'
          : `We could not link ${failed.length} of them. Please try again.`,
      );
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {description ?? (
              <>
                Pick a past meeting to treat as a previous occurrence of this one. Its summary
                feeds this meeting&apos;s prep brief.
              </>
            )}
          </DialogDescription>
        </DialogHeader>

        <Input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter by title…"
          aria-label="Filter meetings by title"
          autoFocus
        />

        {children}

        {linkError && <p className="text-sm text-destructive">{linkError}</p>}

        <div className="max-h-72 overflow-y-auto">
          {loadError ? (
            <p className="px-1 py-4 text-sm text-muted-foreground">{loadError}</p>
          ) : meetings === null ? (
            <div className="flex items-center gap-2 px-1 py-4 text-sm text-muted-foreground">
              <Spinner />
              Loading meetings…
            </div>
          ) : listed.length === 0 ? (
            <p className="px-1 py-4 text-sm text-muted-foreground">
              {filter.trim() ? 'No meetings match that title.' : 'No past meetings to link yet.'}
            </p>
          ) : (
            <ul className="flex flex-col gap-0.5">
              {listed.map((m) => (
                <li key={m.id}>
                  {multiple ? (
                    // The checkbox's own label is the row, so the whole row toggles it
                    // (a checkbox inside a ListRow button would nest buttons).
                    <Checkbox
                      checked={selected.includes(m.id)}
                      onCheckedChange={() => toggle(m.id)}
                      disabled={linkingMany}
                      labelClassName="flex w-full max-w-none rounded-sm px-2 py-1.5 hover:bg-muted [&>span]:flex-1"
                      label={
                        <span className="flex min-w-0 items-baseline justify-between gap-3">
                          <span className="u-row-title truncate">
                            {m.title?.trim() || 'Untitled meeting'}
                          </span>
                          {formatMeetingDate(m.createdAt) && (
                            <span className="u-row-meta shrink-0">
                              {formatMeetingDate(m.createdAt)}
                            </span>
                          )}
                        </span>
                      }
                    />
                  ) : (
                  /* While one link is in flight every row is inert (the row primitive has
                      no `disabled`, so the click is ignored instead). */
                  <ListRow
                    onClick={() => {
                      if (linkingId === null) void link(m.id);
                    }}
                    lead={linkingId === m.id ? <Spinner className="text-brand" /> : undefined}
                    title={m.title?.trim() || 'Untitled meeting'}
                    trailing={
                      formatMeetingDate(m.createdAt) ? (
                        <span className="u-row-meta">{formatMeetingDate(m.createdAt)}</span>
                      ) : undefined
                    }
                    className={cn(linkingId !== null && 'opacity-60')}
                  />
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>

        {multiple && (
          <DialogFooter>
            <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={linkingMany}>
              Cancel
            </Button>
            <Button
              onClick={() => void linkSelected()}
              disabled={selected.length === 0 || linkingMany}
            >
              {linkingMany && <Spinner />}
              {confirmLabel(selected.length)}
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
