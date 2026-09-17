import { useEffect, useRef, useState } from 'react';

export type MeetingTabKey = 'summary' | 'transcript' | 'notes' | 'prep';

export interface MeetingTab {
  key: MeetingTabKey;
  label: string;
}

interface UseMeetingTabsParams {
  /** Scheduled meetings (specs/0036): upcoming occurrences with no recording yet. */
  isScheduled: boolean;
  /** Notes-only meetings (spec 0015): no recording/transcript/audio. */
  isNotesOnly: boolean;
  /** The Today view deep-links to `?tab=prep` to open the Prep tab directly. */
  wantsPrepTab: boolean;
  /** specs/0033 — search deep-link: open the Transcript tab and scroll to this segment. */
  deepLinkSegmentId?: string | null;
  /** specs/0033 — consume the deep-link intent (scroll done or impossible). */
  onDeepLinkConsumed?: () => void;
  /** specs/0060 — screenshot pipeline deep-link: `/meeting-details?tab=<key>` seeds the
   *  active tab directly. An unrecognized value falls back to the normal default rules. */
  requestedTab?: MeetingTabKey | null;
}

const VALID_MEETING_TAB_KEYS: MeetingTabKey[] = ['summary', 'transcript', 'notes', 'prep'];

/**
 * Tab state for the meeting-details single-column layout (Summary / Transcript /
 * My notes / Prep): which tabs exist for this meeting kind, the active tab (with the
 * deep-link / scheduled / notes-only initial-tab rules), lazy Prep mounting, and the
 * WAI-ARIA roving-focus keyboard handling for the tablist.
 */
export function useMeetingTabs({
  isScheduled,
  isNotesOnly,
  wantsPrepTab,
  deepLinkSegmentId,
  onDeepLinkConsumed,
  requestedTab,
}: UseMeetingTabsParams) {
  // Active document tab in the single-column layout (Summary / Transcript / My notes / Prep).
  // Precedence: a `?segment=` deep-link (specs/0033, the effect below) wins over
  // everything — the segment only means something once the Transcript tab is actually
  // visible, so it force-switches there even if something else requested a different
  // tab. Below that, an explicit `requestedTab` (specs/0060's `?tab=` screenshot
  // deep-link) seeds the initial tab; an unrecognized value is ignored and falls
  // through to the legacy defaults: `?tab=prep` / a scheduled meeting → Prep
  // (specs/0036), notes-only → My notes, else Summary.
  const seededTab =
    requestedTab && VALID_MEETING_TAB_KEYS.includes(requestedTab) ? requestedTab : null;
  const [activeTab, setActiveTab] = useState<MeetingTabKey>(
    seededTab ??
      (wantsPrepTab || isScheduled
        ? 'prep'
        : isNotesOnly
          ? 'notes'
          : deepLinkSegmentId
            ? 'transcript'
            : 'summary'),
  );

  // Lazy-mount the Prep tab: only build it once the user actually opens it (or it's the
  // default for a scheduled/deep-linked meeting), so opening a recorded meeting on the
  // Summary tab never kicks off prep-brief generation nobody asked for. Once opened it
  // stays mounted to preserve loaded state (specs/0036).
  const [hasOpenedPrep, setHasOpenedPrep] = useState(activeTab === 'prep');
  useEffect(() => {
    if (activeTab === 'prep') setHasOpenedPrep(true);
  }, [activeTab]);

  // specs/0033 — a segment deep-link arriving while this page is already mounted
  // (e.g. ⌘K search from the same meeting) still switches to the Transcript tab.
  // The scroll itself is gated on the tab actually being VISIBLE (see
  // isScrollTargetVisible in the transcript panel wiring): the tabpanels stay mounted
  // display:none, and the child's scroll effect flushes before this parent effect flips
  // the tab — scrolling a zero-height hidden subtree would silently do nothing.
  // Notes-only meetings have no Transcript tab at all, so the intent is consumed
  // immediately (URL cleaned). This runs on mount too (deepLinkSegmentId is already
  // set in the initial props), so it overrides whatever `requestedTab` (specs/0060)
  // seeded activeTab to above — `?tab=summary&segment=X` still ends on Transcript.
  useEffect(() => {
    if (!deepLinkSegmentId) return;
    if (isNotesOnly) {
      onDeepLinkConsumed?.();
      return;
    }
    setActiveTab('transcript');
  }, [deepLinkSegmentId, isNotesOnly, onDeepLinkConsumed]);

  // specs/0036 — the Prep tab. For a scheduled (upcoming) meeting it's the ONLY
  // content tab besides My notes (there's no recording yet). For a recorded/
  // notes-only meeting it's appended AFTER the existing tabs and labelled to read
  // as pre-meeting context ("Prep (before)"), kept secondary to Summary.
  const tabs: MeetingTab[] =
    isScheduled
      ? [
          { key: 'prep', label: 'Prep' },
          { key: 'notes', label: 'My notes' },
        ]
      : isNotesOnly
        ? [
            // Notes-only: no Transcript tab. Stable order with recorded meetings
            // (Summary first); the default active tab is 'notes' (see activeTab init).
            { key: 'summary', label: 'Summary' },
            { key: 'notes', label: 'My notes' },
            { key: 'prep', label: 'Prep (before)' },
          ]
        : [
            { key: 'summary', label: 'Summary' },
            { key: 'transcript', label: 'Transcript' },
            { key: 'notes', label: 'My notes' },
            { key: 'prep', label: 'Prep (before)' },
          ];

  // Roving-focus support for the tablist (WAI-ARIA tabs pattern): arrow/Home/End move focus
  // and activate the target tab (spec 0028, Low — a11y).
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

  return { activeTab, setActiveTab, hasOpenedPrep, tabs, tabRefs, handleTabKeyDown };
}
