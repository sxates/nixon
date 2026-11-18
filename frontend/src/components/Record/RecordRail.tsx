'use client';

/**
 * The record screen's right-hand rail (specs/0056 W4): one column with
 * **Notes** / **Prep** tabs.
 *
 * Replaces the 0054 prep slide-over (`PrepDrawer`) + header button. The drawer
 * had no side gutter and covered the transcript; the owner wanted prep to live
 * alongside the notepad instead. Everything prep-related still comes from
 * `PrepPanel` unchanged — this only owns the tab chrome and panel lifecycle.
 *
 * Lifecycle rules that matter:
 * - The Notes panel (`RecordAgendaPanel` + `NotepadPanel`) is ALWAYS mounted and
 *   merely hidden while Prep is shown, so the notepad's debounced autosave and
 *   unmount flush (`NotepadPanel.test.tsx`) are untouched by tab switches.
 * - `PrepPanel` mounts lazily on first activation and then stays mounted — its
 *   `api_get_prep` read kicks off brief generation when absent, which nobody asked
 *   for during a recording they never opened prep on (same rule as meeting-details).
 * - The Prep tab exists whenever there is a meeting row; before that (permissions
 *   pending, recording not yet started) only Notes shows.
 *
 * `view` (owner feedback, v0.18) restyles this column. Transcript: the right-hand rail, with
 * the Topics row above My Notes / Agenda / To-Dos tabs. My Notes and Agenda (the header's
 * other two views): the full-width main column, showing that one panel with no sub-tabs (the
 * header's tabs choose it), under a strip of the same height that carries Participants and
 * the summary prompt; the Topics row and To-Dos move to the page's right-hand sidebar. Every
 * panel keeps its place in the tree in all three views, so the notepad keeps its editor, its
 * undo history and its caret, and there is only ever one PrepPanel. A rail left on To-Dos
 * shows My Notes in the other views and returns to To-Dos with Transcript.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import dynamic from 'next/dynamic';
import { PrepPanel } from '@/components/MeetingDetails/PrepPanel';
import { RecordAgendaPanel } from '@/components/Record/RecordAgendaPanel';
import { RecordTodos } from '@/components/Record/RecordTodoRail';
import { RecordTopicsStrip } from '@/components/Record/RecordTopicsStrip';
import { usePrepAvailability } from '@/hooks/usePrepAvailability';
import { Badge } from '@/components/ui/badge';
import { isMainColumnView, RECORD_STRIP_HEIGHT, type RecordView } from '@/lib/record-view';
import { cn } from '@/lib/utils';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';

// Client-only: NotepadPanel's editor is a contentEditable that touches `document`,
// which must not run during SSR. ssr:false keeps it off the server render path.
const NotepadPanel = dynamic(
  () => import('@/app/_components/NotepadPanel').then((m) => m.NotepadPanel),
  { ssr: false },
);

type RecordTabKey = 'notes' | 'prep' | 'todos';

interface RecordTab {
  key: RecordTabKey;
  label: string;
}

interface RecordRailProps {
  meetingId: string | null;
  /** Transcript (default): the right-hand rail. My Notes / Agenda: the full-width main column. */
  view?: RecordView;
  /** The live title, for the Topics row's Series option. */
  meetingTitle?: string | null;
  /** Keys at the top strip's far right (My Notes / Agenda: Participants + summary prompt). */
  tabsTrailing?: ReactNode;
}

export function RecordRail({ meetingId, view = 'transcript', meetingTitle = null, tabsTrailing }: RecordRailProps) {
  const [activeTab, setActiveTab] = useState<RecordTabKey>('notes');
  const { openItemCount, refresh: refreshPrepCount } = usePrepAvailability(meetingId);

  const showPrep = !!meetingId;
  // My Notes / Agenda views: the main column, one panel, chosen by the header's tabs.
  const mainColumn = isMainColumnView(view);
  // To-Dos is a tab in Transcript only: the other views show them in their own sidebar.
  const showTodos = !!meetingId && !mainColumn;
  const tabs: RecordTab[] = [
    { key: 'notes', label: 'My Notes' },
    ...(showPrep ? [{ key: 'prep' as const, label: 'Agenda' }] : []),
    ...(showTodos ? [{ key: 'todos' as const, label: 'To-Dos' }] : []),
  ];

  // A tab that isn't offered right now (no meeting row yet, or To-Dos outside Transcript)
  // falls back to Notes, so the tablist always has an active tab; the choice itself is kept.
  // My Notes / Agenda views pick the panel themselves (Agenda falls back to Notes too while
  // there is no meeting row; the header doesn't offer it then).
  const effectiveTab: RecordTabKey =
    view === 'notes'
      ? 'notes'
      : view === 'agenda'
        ? showPrep
          ? 'prep'
          : 'notes'
        : tabs.some((t) => t.key === activeTab)
          ? activeTab
          : 'notes';

  // Lazy-mount Prep on first showing (from either the rail tab or the Agenda view); keep it
  // mounted afterwards so the loaded brief / open items persist across switches (mirrors
  // useMeetingTabs). One instance serves both, so brief generation starts at most once.
  const [hasOpenedPrep, setHasOpenedPrep] = useState(false);
  useEffect(() => {
    if (effectiveTab === 'prep') setHasOpenedPrep(true);
  }, [effectiveTab]);
  // To-Dos likewise mount on first activation and stay (hidden) while in Transcript.
  const [hasOpenedTodos, setHasOpenedTodos] = useState(false);
  useEffect(() => {
    if (effectiveTab === 'todos') setHasOpenedTodos(true);
  }, [effectiveTab]);

  // Roving tabindex (WAI-ARIA tabs pattern): arrow/Home/End move focus and activate.
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const handleTabKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    let nextIndex: number | null = null;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') nextIndex = (index + 1) % tabs.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') nextIndex = (index - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') nextIndex = 0;
    else if (e.key === 'End') nextIndex = tabs.length - 1;
    if (nextIndex === null) return;
    e.preventDefault();
    setActiveTab(tabs[nextIndex].key);
    tabRefs.current[nextIndex]?.focus();
  };

  return (
    <div
      data-record-view={view}
      className={cn(
        'min-w-0 flex flex-col overflow-hidden',
        mainColumn ? 'flex-1' : 'w-[40%] max-w-[440px] border-l border-border',
      )}
    >
      {/* Transcript: the Topics row heads the rail. Elsewhere it heads the To-Do sidebar. */}
      {view === 'transcript' && <RecordTopicsStrip meetingId={meetingId} meetingTitle={meetingTitle} />}
      {/* The `ui/tabs` underline strip (the same one as meeting details). Only the strip is
          Radix: the panels below keep their own lifecycle (always-mounted Notes, lazy Prep),
          so each trigger names its panel explicitly. The arrow-key handler is ours and calls
          `preventDefault`, which Radix's own roving focus defers to. */}
      {/* The top strip is RECORD_STRIP_HEIGHT tall with its rule at the bottom, so it lines up
          with the Topics strip beside it (owner feedback, v0.18). Transcript: the tab strip.
          My Notes / Agenda: no sub-tabs, just a quiet label naming the panel (an empty band
          before a recording read as broken) and `tabsTrailing` at the far right. px-2 + the
          tab's px-3, or px-5, puts the first label on the panels' gutter line; pr-5 puts the
          last key on it too. Both strips occupy the same slot, so nothing below shifts. */}
      {mainColumn ? (
        <div
          data-testid="record-rail-strip"
          className={cn('flex flex-shrink-0 items-center gap-3 border-b border-border bg-background pl-5 pr-5', RECORD_STRIP_HEIGHT)}
        >
          <h2 className="u-section-label min-w-0 flex-1 truncate">{effectiveTab === 'prep' ? 'Agenda' : 'My Notes'}</h2>
          {tabsTrailing ? <div className="flex min-w-0 items-center">{tabsTrailing}</div> : null}
        </div>
      ) : (
        <Tabs
          value={effectiveTab}
          onValueChange={(value) => setActiveTab(value as RecordTabKey)}
          data-testid="record-rail-strip"
          className={cn('flex flex-shrink-0 items-stretch gap-3 border-b border-border bg-background pl-2 pr-5', RECORD_STRIP_HEIGHT)}
        >
          <TabsList
            aria-label="Recording side panel"
            className="flex-shrink-0 items-end border-b-0"
          >
            {tabs.map(({ key, label }, index) => (
              <TabsTrigger
                key={key}
                value={key}
                ref={(el) => {
                  tabRefs.current[index] = el;
                }}
                id={`record-tab-${key}`}
                aria-controls={`record-tabpanel-${key}`}
                onClick={() => setActiveTab(key)}
                onKeyDown={(e) => handleTabKeyDown(e, index)}
                badge={
                  key === 'prep' && openItemCount > 0 ? (
                    <Badge
                      tone="brand"
                      className="px-1 py-0"
                      aria-label={`${openItemCount} carried-over open item${openItemCount === 1 ? '' : 's'}`}
                    >
                      {openItemCount}
                    </Badge>
                  ) : null
                }
              >
                {label}
              </TabsTrigger>
            ))}
          </TabsList>
          {tabsTrailing ? <div className="ml-auto flex min-w-0 items-center">{tabsTrailing}</div> : null}
        </Tabs>
      )}

      {/* NOTES — pinned agenda strip + live notepad. Always mounted (see header comment). */}
      <div
        role={mainColumn ? undefined : 'tabpanel'}
        id="record-tabpanel-notes"
        aria-labelledby={mainColumn ? undefined : 'record-tab-notes'}
        hidden={effectiveTab !== 'notes'}
        className={effectiveTab === 'notes' ? 'min-h-0 flex flex-1 flex-col overflow-hidden' : 'hidden'}
      >
        {meetingId && <RecordAgendaPanel meetingId={meetingId} />}
        {/* No "MY NOTES" label: the tab (or the strip) right above already says it. */}
        <NotepadPanel meetingId={meetingId ?? undefined} showHeader={false} />
      </div>

      {/* PREP — lazily mounted on first activation, then kept. px-5 = the record-screen gutter. */}
      {showPrep && (effectiveTab === 'prep' || hasOpenedPrep) && (
        <div
          role={mainColumn ? undefined : 'tabpanel'}
          id="record-tabpanel-prep"
          aria-labelledby={mainColumn ? undefined : 'record-tab-prep'}
          hidden={effectiveTab !== 'prep'}
          className={effectiveTab === 'prep' ? 'min-h-0 flex flex-1 flex-col overflow-hidden' : 'hidden'}
        >
          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
            <PrepPanel
              key={meetingId}
              meetingId={meetingId}
              onOpenItemsChanged={refreshPrepCount}
            />
          </div>
        </div>
      )}

      {/* TO-DOS (Transcript only) — lazily mounted on first activation, then kept while in
          Transcript. My Notes / Agenda unmount it: their sidebar shows the same list. */}
      {showTodos && (effectiveTab === 'todos' || hasOpenedTodos) && (
        <div
          role={mainColumn ? undefined : 'tabpanel'}
          id="record-tabpanel-todos"
          aria-labelledby={mainColumn ? undefined : 'record-tab-todos'}
          hidden={effectiveTab !== 'todos'}
          className={effectiveTab === 'todos' ? 'min-h-0 flex flex-1 flex-col overflow-hidden' : 'hidden'}
        >
          <RecordTodos meetingId={meetingId} heading={false} />
        </div>
      )}
    </div>
  );
}
