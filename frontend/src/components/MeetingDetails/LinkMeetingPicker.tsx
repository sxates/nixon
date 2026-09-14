'use client';

/**
 * LinkMeetingPicker (specs/0041 WS4) — the "Link previous meeting…" dialog on the Prep tab.
 *
 * Lists recent recorded meetings (from the existing `api_get_meetings` list command — the
 * same surface the Home dashboard / ⌘K palette use; `scheduled` prep placeholders are
 * already excluded server-side), filterable by title client-side. Picking one calls
 * `api_link_meeting_to_series`, pinning it into the CURRENT meeting's series so prep briefs
 * and carryover can draw from it.
 */

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Link2, Loader2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
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
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The meeting whose series the picked meeting is linked into. */
  targetMeetingId: string;
  /** Meetings to hide from the list (the target itself + already-linked meetings). */
  excludeIds: string[];
  /** Called after a successful link so the parent can reload its prep view. */
  onLink: () => void;
}) {
  const [meetings, setMeetings] = useState<PickerMeeting[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [linkingId, setLinkingId] = useState<string | null>(null);
  const [linkError, setLinkError] = useState<string | null>(null);

  // (Re)load the recent-meetings list each time the picker opens.
  useEffect(() => {
    if (!open) return;
    setFilter('');
    setLinkError(null);
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
      await invoke('api_link_meeting_to_series', {
        meetingId: id,
        targetMeetingId,
      });
      onOpenChange(false);
      onLink();
    } catch (err) {
      console.error('Failed to link meeting to series:', err);
      setLinkError('We could not link that meeting. Please try again.');
    } finally {
      setLinkingId(null);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Link previous meeting</DialogTitle>
          <DialogDescription>
            Pick a past meeting to treat as a previous occurrence of this one. Its summary
            feeds this meeting&apos;s prep brief.
          </DialogDescription>
        </DialogHeader>

        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter by title…"
          aria-label="Filter meetings by title"
          autoFocus
          className="w-full rounded-lg border border-border bg-card px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />

        {linkError && <p className="text-sm text-destructive">{linkError}</p>}

        <div className="max-h-72 overflow-y-auto">
          {loadError ? (
            <p className="px-1 py-4 text-sm text-muted-foreground">{loadError}</p>
          ) : meetings === null ? (
            <div className="flex items-center gap-2 px-1 py-4 text-sm text-muted-foreground">
              <Loader2 size={14} aria-hidden="true" className="animate-spin" />
              Loading meetings…
            </div>
          ) : listed.length === 0 ? (
            <p className="px-1 py-4 text-sm text-muted-foreground">
              {filter.trim() ? 'No meetings match that title.' : 'No past meetings to link yet.'}
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {listed.map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    disabled={linkingId !== null}
                    onClick={() => void link(m.id)}
                    className="flex w-full items-baseline gap-2 rounded-lg px-2 py-2 text-left transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
                  >
                    {linkingId === m.id ? (
                      <Loader2
                        size={13}
                        aria-hidden="true"
                        className="flex-shrink-0 animate-spin self-center text-brand"
                      />
                    ) : (
                      <Link2
                        size={13}
                        aria-hidden="true"
                        className="flex-shrink-0 self-center text-muted-foreground"
                      />
                    )}
                    <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
                      {m.title?.trim() || 'Untitled meeting'}
                    </span>
                    {formatMeetingDate(m.createdAt) && (
                      <span className="flex-shrink-0 text-xs text-muted-foreground">
                        {formatMeetingDate(m.createdAt)}
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
