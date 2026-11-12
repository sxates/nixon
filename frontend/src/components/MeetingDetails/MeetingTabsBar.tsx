'use client';

import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import type { MeetingTab, MeetingTabKey } from '@/hooks/meeting-details/useMeetingTabs';

interface MeetingTabsBarProps {
  tabs: MeetingTab[];
  activeTab: MeetingTabKey;
  onSelect: (key: MeetingTabKey) => void;
  tabRefs: React.MutableRefObject<Array<HTMLButtonElement | null>>;
  onTabKeyDown: (e: React.KeyboardEvent<HTMLButtonElement>, index: number) => void;
}

/**
 * The meeting-details tablist — the `ui/tabs` underline strip. specs/0019 WS1.1:
 * sticky to the top of the scroll column so Summary/Transcript/Notes stay reachable
 * from deep in a long transcript without scrolling back up. bg-background keeps
 * content from showing through as it scrolls under.
 *
 * Only the strip comes from Radix: the panels are `MeetingTabPanels`, which keeps its own
 * `meeting-tabpanel-*` ids and lazy-mount rules, so each trigger names them explicitly.
 * The arrow-key handler is `useMeetingTabs`'s; it calls `preventDefault`, which Radix's
 * own roving focus then defers to.
 */
export function MeetingTabsBar({ tabs, activeTab, onSelect, tabRefs, onTabKeyDown }: MeetingTabsBarProps) {
  return (
    // `contents`: the root draws no box, so the sticky list's containing block is still the
    // whole document column — inside a box of its own height it could never stick.
    <Tabs
      value={activeTab}
      onValueChange={(value) => onSelect(value as MeetingTabKey)}
      className="contents"
    >
      <TabsList aria-label="Meeting views" className="sticky top-0 z-10 mb-3 mt-4 bg-background pt-2">
        {tabs.map(({ key, label }, index) => (
          <TabsTrigger
            key={key}
            value={key}
            ref={(el) => {
              tabRefs.current[index] = el;
            }}
            id={`meeting-tab-${key}`}
            aria-controls={`meeting-tabpanel-${key}`}
            onClick={() => onSelect(key)}
            onKeyDown={(e) => onTabKeyDown(e, index)}
          >
            {label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}
