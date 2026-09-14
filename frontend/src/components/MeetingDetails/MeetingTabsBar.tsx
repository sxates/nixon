'use client';

import type { MeetingTab, MeetingTabKey } from '@/hooks/meeting-details/useMeetingTabs';

interface MeetingTabsBarProps {
  tabs: MeetingTab[];
  activeTab: MeetingTabKey;
  onSelect: (key: MeetingTabKey) => void;
  tabRefs: React.MutableRefObject<Array<HTMLButtonElement | null>>;
  onTabKeyDown: (e: React.KeyboardEvent<HTMLButtonElement>, index: number) => void;
}

/**
 * The meeting-details tablist — underlined active tab in brand. specs/0019 WS1.1:
 * sticky to the top of the scroll column so Summary/Transcript/Notes stay reachable
 * from deep in a long transcript without scrolling back up. bg-background keeps
 * content from showing through as it scrolls under.
 */
export function MeetingTabsBar({ tabs, activeTab, onSelect, tabRefs, onTabKeyDown }: MeetingTabsBarProps) {
  return (
    <div
      role="tablist"
      aria-label="Meeting views"
      className="sticky top-0 z-10 mb-3 mt-4 flex gap-1 border-b border-border bg-background pt-2"
    >
      {tabs.map(({ key, label }, index) => {
        const active = activeTab === key;
        return (
          <button
            key={key}
            ref={(el) => { tabRefs.current[index] = el; }}
            id={`meeting-tab-${key}`}
            role="tab"
            type="button"
            aria-selected={active}
            aria-controls={`meeting-tabpanel-${key}`}
            // Roving tabindex: only the active tab is in the Tab order; arrow keys move
            // between tabs (WAI-ARIA tabs pattern).
            tabIndex={active ? 0 : -1}
            onClick={() => onSelect(key)}
            onKeyDown={(e) => onTabKeyDown(e, index)}
            className={
              'border-b-2 px-3 py-1.5 text-sm font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ' +
              (active
                ? '-mb-px border-brand text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground')
            }
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}
