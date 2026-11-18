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
 * `view` (owner feedback, v0.18) changes only this column's classes: the right-hand rail
 * in Transcript, the full-width notes column in Focus. The element tree is the same in both,
 * so the notepad keeps its editor, its undo history and its caret across a switch. Above the
 * tabs, once the meeting row exists, sits the Topics row (add this Meeting to a Topic while
 * it records) and, in Transcript, the Add To-Do key that opens ⌘⇧T quick add; in Focus the
 * To-Do rail's own add field does that job.
 */

import { useEffect, useRef, useState } from 'react';
import dynamic from 'next/dynamic';
import { PrepPanel } from '@/components/MeetingDetails/PrepPanel';
import { RecordAgendaPanel } from '@/components/Record/RecordAgendaPanel';
import { openQuickAdd } from '@/components/QuickAdd/QuickAddShortcut';
import { MeetingTopicsRow } from '@/components/Topics/MeetingTopicsRow';
import { usePrepAvailability } from '@/hooks/usePrepAvailability';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Plus } from '@/components/ui/icons';
import type { RecordView } from '@/lib/record-view';
import { cn } from '@/lib/utils';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';

// Client-only: NotepadPanel's editor is a contentEditable that touches `document`,
// which must not run during SSR. ssr:false keeps it off the server render path.
const NotepadPanel = dynamic(
  () => import('@/app/_components/NotepadPanel').then((m) => m.NotepadPanel),
  { ssr: false },
);

type RecordTabKey = 'notes' | 'prep';

interface RecordTab {
  key: RecordTabKey;
  label: string;
}

interface RecordRailProps {
  meetingId: string | null;
  /** Transcript (default): the right-hand rail. Focus: the full-width notes column. */
  view?: RecordView;
  /** The live title, for the Topics row's Series option. */
  meetingTitle?: string | null;
}

export function RecordRail({ meetingId, view = 'transcript', meetingTitle = null }: RecordRailProps) {
  const [activeTab, setActiveTab] = useState<RecordTabKey>('notes');
  const { openItemCount, refresh: refreshPrepCount } = usePrepAvailability(meetingId);

  const tabs: RecordTab[] = meetingId
    ? [
        { key: 'notes', label: 'My Notes' },
        { key: 'prep', label: 'Agenda' },
      ]
    : [{ key: 'notes', label: 'My Notes' }];

  // Lazy-mount Prep on first activation; keep it mounted afterwards so the loaded
  // brief / open items persist across switches (mirrors useMeetingTabs).
  const [hasOpenedPrep, setHasOpenedPrep] = useState(false);
  useEffect(() => {
    if (activeTab === 'prep') setHasOpenedPrep(true);
  }, [activeTab]);

  // If the meeting id disappears (rail shown before a recording starts, or after
  // stop), the Prep tab is gone — fall back to Notes so the tablist always has an
  // active tab.
  const showPrep = !!meetingId;
  const effectiveTab: RecordTabKey = showPrep ? activeTab : 'notes';

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
        view === 'focus' ? 'flex-1' : 'w-[40%] max-w-[440px] border-l border-border',
      )}
    >
      {meetingId && (
        <div className="flex-shrink-0 border-b border-border px-5 py-2">
          <MeetingTopicsRow
            meetingId={meetingId}
            title={meetingTitle}
            seriesKey={null}
            className=""
            trailing={
              view === 'transcript' ? (
                <Button variant="outline" size="xs" className="ml-auto" onClick={openQuickAdd}>
                  <Plus />
                  Add To-Do
                </Button>
              ) : null
            }
          />
        </div>
      )}
      {/* The `ui/tabs` underline strip (the same one as meeting details). Only the strip is
          Radix: the panels below keep their own lifecycle (always-mounted Notes, lazy Prep),
          so each trigger names its panel explicitly. The arrow-key handler is ours and calls
          `preventDefault`, which Radix's own roving focus defers to. */}
      <Tabs
        value={effectiveTab}
        onValueChange={(value) => setActiveTab(value as RecordTabKey)}
        className="flex-shrink-0"
      >
        <TabsList
          aria-label="Recording side panel"
          // px-2 + the tab's px-3 puts the first label on the panels' px-5 gutter line.
          className="bg-background px-2 pt-2"
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
      </Tabs>

      {/* NOTES — pinned agenda strip + live notepad. Always mounted (see header comment). */}
      <div
        role="tabpanel"
        id="record-tabpanel-notes"
        aria-labelledby="record-tab-notes"
        hidden={effectiveTab !== 'notes'}
        className={effectiveTab === 'notes' ? 'min-h-0 flex flex-1 flex-col overflow-hidden' : 'hidden'}
      >
        {meetingId && <RecordAgendaPanel meetingId={meetingId} />}
        {/* No "MY NOTES" label: the tab right above already says it. */}
        <NotepadPanel meetingId={meetingId ?? undefined} showHeader={false} />
      </div>

      {/* PREP — lazily mounted on first activation, then kept. px-5 = the record-screen gutter. */}
      {showPrep && (effectiveTab === 'prep' || hasOpenedPrep) && (
        <div
          role="tabpanel"
          id="record-tabpanel-prep"
          aria-labelledby="record-tab-prep"
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
    </div>
  );
}
