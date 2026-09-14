'use client';

import { useState, useEffect, useCallback, type ReactNode } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
// cmdk's own scorer (command-score fuzzy subsequence) — reused by our manual
// filtering pass so behavior is identical on both sides of the backend-search
// threshold (specs/0033).
import { defaultFilter } from 'cmdk';
import { toast } from 'sonner';
import { Mic, LayoutList, Settings, Calendar, AudioLines, FileText, NotebookPen, Sparkles, ListPlus } from 'lucide-react';
import {
  CommandDialog,
  CommandInput,
  CommandList,
  CommandGroup,
  CommandItem,
  CommandSeparator,
  CommandShortcut,
} from '@/components/ui/command';
import { formatMeetingDate } from '@/lib/format-date';

// Shape returned by `api_get_meetings` (mirrors app/page.tsx DashboardMeeting).
// `gist`/`durationSeconds` may be absent.
interface PaletteMeeting {
  id: string;
  title: string;
  createdAt: string; // ISO-8601 UTC
  durationSeconds?: number;
  gist?: string;
  /** Archival reel ordinal (specs/0057): 1-based, oldest first. Absent on legacy DTOs. */
  reelNumber?: number;
}

// Shape returned by `api_search_meetings` (specs/0033 MeetingSearchHit; the Rust
// struct is serialized `rename_all = "camelCase"`, like `api_get_meetings`).
export interface MeetingSearchHit {
  meetingId: string;
  title: string;
  createdAt: string; // ISO-8601 UTC
  source: 'transcript' | 'summary' | 'notes';
  /** Sentinel-delimited excerpt: matches wrapped in \u0001…\u0002 (see renderSnippet). */
  snippet: string;
  /** Best-matching segment — transcript hits only. */
  transcriptId?: string | null;
  rank: number;
}

/**
 * Window event that opens the palette (specs/0029 WS6.1). The palette's open state
 * is internal (toggled by ⌘K); this gives other surfaces — e.g. the home header's
 * "Search meetings" pill — a way to open it without lifting state into a store.
 */
export const OPEN_COMMAND_PALETTE_EVENT = 'nixon:open-command-palette';

/** Open the globally-mounted ⌘K command palette (no-op if it isn't mounted). */
export function openCommandPalette(): void {
  window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT));
}

/** Minimum query length before we hit the FTS backend (specs/0033). */
const MIN_SEARCH_LENGTH = 2;
/** Debounce for the backend search while typing (specs/0033, decided 200 ms). */
const SEARCH_DEBOUNCE_MS = 200;

/** Snippet sentinel delimiters emitted by the backend around each match (specs/0033). */
const MARK_START = '\u0001';
const MARK_END = '\u0002';

/**
 * Convert a sentinel-delimited FTS snippet into React nodes, rendering each
 * \u0001…\u0002 span as <mark>. The backend never sends HTML and we never use
 * dangerouslySetInnerHTML — plain text in, React elements out (specs/0033).
 * Unpaired/stray sentinels are stripped rather than rendered as control characters.
 */
export function renderSnippet(snippet: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let key = 0;
  const pushPlain = (text: string) => {
    const cleaned = text.split(MARK_START).join('').split(MARK_END).join('');
    if (cleaned) nodes.push(cleaned);
  };
  let rest = snippet;
  for (;;) {
    const start = rest.indexOf(MARK_START);
    if (start === -1) break;
    const end = rest.indexOf(MARK_END, start + 1);
    if (end === -1) break;
    pushPlain(rest.slice(0, start));
    const marked = rest.slice(start + 1, end);
    if (marked) {
      nodes.push(
        <mark key={key++} className="rounded-[2px] bg-brand/15 px-px font-medium text-foreground">
          {marked}
        </mark>,
      );
    }
    rest = rest.slice(end + 1);
  }
  pushPlain(rest);
  return nodes;
}

// Icon per search-hit source (specs/0033).
function SourceIcon({ source }: { source: MeetingSearchHit['source'] }) {
  if (source === 'summary') return <FileText />;
  if (source === 'notes') return <NotebookPen />;
  return <AudioLines />;
}

export default function CommandPalette() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [meetings, setMeetings] = useState<PaletteMeeting[]>([]);
  // Controlled cmdk input — drives the debounced backend search (specs/0033).
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<MeetingSearchHit[]>([]);

  const trimmedQuery = query.trim();
  // While a backend query is active we take over filtering: cmdk cannot mix
  // client-filtered items with server-provided hits (specs/0033).
  const backendSearchActive = trimmedQuery.length >= MIN_SEARCH_LENGTH;

  // ⌘K / Ctrl+K toggles the palette. In-app window keydown listener (not a
  // global system shortcut). preventDefault stops the browser "find" handler.
  // The custom open event (specs/0029 WS6.1) lets UI affordances like the home
  // header's "Search meetings" pill open it too.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((prev) => !prev);
      }
    };
    const onOpenEvent = () => setOpen(true);
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener(OPEN_COMMAND_PALETTE_EVENT, onOpenEvent);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener(OPEN_COMMAND_PALETTE_EVENT, onOpenEvent);
    };
  }, []);

  // Fetch meetings on EVERY palette open — a session-long cache went stale as soon
  // as a meeting was recorded after the first open (and latched even on a failed
  // fetch), while the FTS tier stayed fresh. It's a cheap local invoke; the prior
  // list stays rendered while the refresh is in flight, and a failure keeps it and
  // simply retries on the next open. Filtering is client-side via cmdk.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      try {
        const result = (await invoke('api_get_meetings', { authToken: null })) as PaletteMeeting[];
        if (!cancelled) setMeetings(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('[CommandPalette] Failed to load meetings:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  // Reset the query between palette sessions (the dialog content unmounts on close,
  // so the input clears — keep our controlled state in step with that).
  useEffect(() => {
    if (!open) {
      setQuery('');
      setHits([]);
    }
  }, [open]);

  // Debounced full-text search over transcripts/summaries/notes (specs/0033).
  // Failures fall back to the title-only tier — the palette never breaks.
  useEffect(() => {
    if (!open || !backendSearchActive) {
      setHits([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = (await invoke('api_search_meetings', {
          query: trimmedQuery,
        })) as MeetingSearchHit[];
        if (!cancelled) setHits(Array.isArray(result) ? result : []);
      } catch (err) {
        console.warn('[CommandPalette] Full-text search unavailable, falling back to titles:', err);
        if (!cancelled) setHits([]);
      }
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [open, backendSearchActive, trimmedQuery]);

  // Manual matcher used only while cmdk's own filtering is off (backend search
  // active). It runs cmdk's exported defaultFilter (command-score fuzzy
  // subsequence) over the SAME value+keywords surface cmdk itself would score, so
  // items never vanish at the 2-char backend threshold ('s' and 'st' both keep
  // Settings) and a pasted meeting id still matches its title row.
  const manualMatch = useCallback(
    (value: string, keywords: string[] = []) => {
      if (!backendSearchActive) return true; // cmdk filters
      return defaultFilter(value, trimmedQuery, keywords) > 0;
    },
    [backendSearchActive, trimmedQuery],
  );

  // Run an action and close the palette.
  const runCommand = useCallback((action: () => void) => {
    setOpen(false);
    action();
  }, []);

  const startRecording = useCallback(() => {
    // Match the dashboard Record button: navigate to the recording route.
    router.push('/record');
  }, [router]);

  // WS1.d (specs/0038) — "New to-do": when the query holds text, create a standalone
  // manual action item from it (no meeting) and toast a link to the hub; an empty query
  // just opens the hub, where the add row is focused.
  const createTodo = useCallback(async () => {
    const description = trimmedQuery;
    if (!description) {
      router.push('/tasks');
      return;
    }
    try {
      await invoke('api_create_action_item', { meetingId: null, description });
      toast.success('To-do added', {
        description: 'Find it in Action items.',
        action: { label: 'Open', onClick: () => router.push('/tasks') },
      });
    } catch (err) {
      console.error('Failed to create to-do:', err);
      toast.error('Could not add the to-do', {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  }, [router, trimmedQuery]);

  // Static actions tier — data-driven so the manual (shouldFilter=false) pass can
  // filter it the same way cmdk would. Empty-query rendering is unchanged.
  // (The always-visible Ask-AI action lives in its own group at the END of the
  // list — see below — so it never steals cmdk's first-item auto-selection from
  // the top search result.)
  const actions: Array<{
    label: string;
    keywords: string[];
    icon: ReactNode;
    run: () => void;
    shortcut?: string;
  }> = [
    {
      label: 'Start recording',
      keywords: ['record', 'start', 'capture', 'meeting'],
      icon: <Mic />,
      run: startRecording,
      shortcut: '⌘⇧R',
    },
    {
      label: 'Go to Meetings',
      keywords: ['home', 'dashboard', 'meetings', 'list'],
      icon: <LayoutList />,
      run: () => router.push('/'),
    },
    {
      label: 'Settings',
      keywords: ['settings', 'preferences', 'config', 'options'],
      icon: <Settings />,
      run: () => router.push('/settings'),
    },
  ];

  const visibleActions = actions.filter((a) => manualMatch(a.label, a.keywords));
  const visibleMeetings = meetings.filter((m) => {
    const title = m.title?.trim() || 'Untitled meeting';
    // Same surface as the CommandItem below (value=`${title} ${id}`, keywords=[title])
    // so pasted meeting ids match here exactly like they do under cmdk filtering.
    return manualMatch(`${title} ${m.id}`, [title]);
  });

  return (
    <CommandDialog open={open} onOpenChange={setOpen} shouldFilter={!backendSearchActive}>
      <CommandInput
        placeholder="Search meetings or run a command..."
        value={query}
        onValueChange={setQuery}
      />
      <CommandList>
        {visibleActions.length > 0 && (
          <CommandGroup heading="Actions">
            {visibleActions.map((a) => (
              <CommandItem
                key={a.label}
                onSelect={() => runCommand(a.run)}
                keywords={a.keywords}
              >
                {a.icon}
                <span>{a.label}</span>
                {a.shortcut && <CommandShortcut>{a.shortcut}</CommandShortcut>}
              </CommandItem>
            ))}
          </CommandGroup>
        )}

        {visibleMeetings.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Meetings">
              {visibleMeetings.map((m) => {
                const title = m.title?.trim() || 'Untitled meeting';
                const date = formatMeetingDate(m.createdAt);
                return (
                  <CommandItem
                    key={m.id}
                    // cmdk filters by `value`; use the title so it matches typed text.
                    value={`${title} ${m.id}`}
                    keywords={[title]}
                    onSelect={() => runCommand(() => router.push(`/meeting-details?id=${m.id}`))}
                  >
                    <Calendar />
                    <span className="min-w-0 flex-1 truncate">{title}</span>
                    {date && (
                      <span className="ml-auto flex-shrink-0 text-xs text-muted-foreground">
                        {date}
                      </span>
                    )}
                  </CommandItem>
                );
              })}
            </CommandGroup>
          </>
        )}

        {/* Full-text hits over transcripts/summaries/notes (specs/0033). Only present
            while a backend search is active; deep-links to the meeting (and segment
            for transcript hits). */}
        {backendSearchActive && hits.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="In transcripts, summaries & notes">
              {hits.map((h) => {
                const title = h.title?.trim() || 'Untitled meeting';
                const date = formatMeetingDate(h.createdAt);
                const href =
                  `/meeting-details?id=${h.meetingId}` +
                  (h.transcriptId ? `&segment=${h.transcriptId}` : '');
                return (
                  <CommandItem
                    key={`${h.meetingId}-${h.source}`}
                    value={`search-hit ${h.meetingId} ${h.source}`}
                    onSelect={() => runCommand(() => router.push(href))}
                  >
                    <SourceIcon source={h.source} />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-baseline gap-2">
                        <span className="min-w-0 flex-1 truncate">{title}</span>
                        {date && (
                          <span className="ml-auto flex-shrink-0 text-xs text-muted-foreground">
                            {date}
                          </span>
                        )}
                      </div>
                      <p className="truncate text-xs text-muted-foreground">
                        {renderSnippet(h.snippet)}
                      </p>
                    </div>
                  </CommandItem>
                );
              })}
            </CommandGroup>
          </>
        )}

        {/* Scoped empty state (specs/0035): cmdk's CommandEmpty counts only
            store-registered items, so it would show "No results found." right
            next to the always-visible (forceMount) Ask-AI row — a contradictory
            empty state. Instead we condition on the palette's OWN result state:
            the message renders only when a backend search matched nothing at
            all, and it says what's actually empty. */}
        {backendSearchActive &&
          visibleActions.length === 0 &&
          visibleMeetings.length === 0 &&
          hits.length === 0 && (
            <div className="py-6 text-center text-sm text-muted-foreground">
              No matching meetings
            </div>
          )}

        {/* Cross-meeting Ask-AI (specs/0035): whatever is typed rides along as
            the pre-filled question on /ask ("what did we decide about X" → Ask
            page). Always visible — the typed text IS its input, so a natural-
            language question must never filter it out (forceMount keeps it
            rendered while cmdk filters short queries). It sits in its own LAST
            group so cmdk's first-DOM-item auto-selection stays on the top
            search result and type-then-Enter (specs/0033) keeps opening it. */}
        <CommandSeparator />
        <CommandGroup>
          <CommandItem
            forceMount
            onSelect={() =>
              runCommand(() =>
                router.push(trimmedQuery ? `/ask?q=${encodeURIComponent(trimmedQuery)}` : '/ask'),
              )
            }
          >
            <Sparkles />
            <span>Ask AI about your meetings…</span>
          </CommandItem>
          <CommandItem forceMount onSelect={() => runCommand(() => void createTodo())}>
            <ListPlus />
            <span>{trimmedQuery ? `New to-do: “${trimmedQuery}”` : 'New to-do…'}</span>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
