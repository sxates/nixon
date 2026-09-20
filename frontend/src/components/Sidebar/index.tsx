'use client';

import React, { useEffect } from 'react';
import {
  CalendarDays,
  CheckSquare,
  Home,
  Menu,
  MessageSquare,
  Settings,
  Users,
  type LucideIcon,
} from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import { DeckIcon } from '@/components/ui/deck-icon';
import { cn } from '@/lib/utils';
import { QueueRow } from './QueueRow';
import { IconSlot, NavRow } from './SidebarRow';
import { SIDEBAR_LABEL, SIDEBAR_ROW } from './row';
import { UpdateRow } from './UpdateRow';

import DevBadge from '../DevBadge';

/** One entry in the primary navigation. */
interface NavItem {
  label: string;
  path: string;
  /** The silkscreened glyph for this destination (specs/0057 §2). */
  icon: LucideIcon;
  /** The `data-sidebar-slot` name this destination renders (specs/0069 W1). */
  slot: 'today' | 'meetings' | 'tasks' | 'people' | 'ask';
  /** Returns true when this item should render as active for the current path. */
  isActive: (pathname: string | null) => boolean;
}

const NAV_ITEMS: NavItem[] = [
  {
    label: 'Today',
    path: '/',
    icon: Home,
    slot: 'today',
    isActive: (p) => p === '/',
  },
  {
    label: 'All meetings',
    // The meeting-details view is the destination of every meeting row, so it
    // belongs to the "All meetings" section for active-state purposes.
    path: '/meetings',
    icon: CalendarDays,
    slot: 'meetings',
    isActive: (p) => (p?.startsWith('/meetings') ?? false) || (p?.startsWith('/meeting-details') ?? false),
  },
  {
    // Cross-meeting task hub (specs/0034).
    label: 'Action items',
    path: '/tasks',
    icon: CheckSquare,
    slot: 'tasks',
    isActive: (p) => p?.startsWith('/tasks') ?? false,
  },
  {
    label: 'People',
    path: '/people',
    icon: Users,
    slot: 'people',
    isActive: (p) => p?.startsWith('/people') ?? false,
  },
  {
    // Cross-meeting Ask-AI (specs/0035).
    label: 'Ask AI',
    path: '/ask',
    icon: MessageSquare,
    slot: 'ask',
    isActive: (p) => p?.startsWith('/ask') ?? false,
  },
];

/** The brushed-panel surface shared by both sidebar branches (specs/0057 decision 5). */
const PANEL_SURFACE =
  'bg-panel shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_-1px_0_0_hsl(var(--bevel-lo))]';

const Sidebar: React.FC = () => {
  const router = useRouter();
  const pathname = usePathname();
  const { isCollapsed, toggleCollapse } = useSidebar();

  // Expose openSettings to window for the Rust tray to call (preserved from the
  // previous sidebar — the tray invokes this to surface settings).
  useEffect(() => {
    const win = window as Window & { openSettings?: () => void };
    win.openSettings = () => {
      router.push('/settings');
    };
    return () => {
      delete win.openSettings;
    };
  }, [router]);

  // Collapsed and expanded are ONE tree (specs/0069 W1): every row shares the same height,
  // icon column and glyph size (`row.ts`), so opening the panel only ever changes its width
  // and whether labels render — nothing moves or shrinks.
  return (
    <div className="fixed top-0 left-0 z-40 h-screen">
      <div
        className={cn(
          'relative flex h-screen flex-col border-r border-border transition-all duration-300',
          isCollapsed ? 'w-16' : 'w-64',
          PANEL_SURFACE,
        )}
      >
        <WalnutCheek />

        {/* Hamburger — a row like any other, so the glyph does not move when the panel
            opens. It carries no label: the panel's own width says which way it goes. */}
        <button
          type="button"
          onClick={toggleCollapse}
          aria-label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          aria-expanded={!isCollapsed}
          className={cn(
            SIDEBAR_ROW,
            'mt-2 flex-shrink-0 text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
          )}
        >
          <IconSlot name="menu">
            <DeckIcon icon={Menu} size={18} />
          </IconSlot>
        </button>

        {/* The reel mark, in both states. Expanded it gains NIXON as its label rather than
            being swapped for a wordmark that sits somewhere else. */}
        <div className={cn(SIDEBAR_ROW, 'flex-shrink-0')}>
          <IconSlot name="mark">
            <NixonMark />
          </IconSlot>
          {!isCollapsed && (
            <span className={cn(SIDEBAR_LABEL, 'text-[13px] tracking-[0.18em] text-foreground')}>
              NIXON
            </span>
          )}
        </div>

        <nav className="mt-2 flex flex-1 flex-col overflow-y-auto">
          {NAV_ITEMS.map((item) => (
            <NavRow
              key={item.label}
              slot={item.slot}
              icon={item.icon}
              label={item.label}
              collapsed={isCollapsed}
              active={item.isActive(pathname)}
              onClick={() => router.push(item.path)}
            />
          ))}
        </nav>

        <div className="flex-shrink-0 py-2">
          <UpdateRow collapsed={isCollapsed} />
          <NavRow
            slot="settings"
            icon={Settings}
            label="Settings"
            collapsed={isCollapsed}
            active={pathname?.startsWith('/settings') ?? false}
            onClick={() => router.push('/settings')}
          />
          <QueueRow collapsed={isCollapsed} />
          {process.env.NODE_ENV !== 'production' && (
            <div className={cn(SIDEBAR_ROW, 'mb-1')}>
              <IconSlot name="dev">
                <DevBadge />
              </IconSlot>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

/** The walnut cheek running down the machine's left edge (specs/0057 decision 5). */
function WalnutCheek() {
  return <span aria-hidden className="absolute inset-y-0 left-0 z-10 w-1.5 bg-walnut" />;
}

/** The ⊙—⊙ reel mark. Renders in both states — collapsed it stands alone; expanded it sits
 *  beside the NIXON label rather than being swapped out for it. */
function NixonMark() {
  return (
    <svg
      viewBox="0 0 32 16"
      width="28"
      height="14"
      role="img"
      aria-label="Nixon"
      fill="none"
      className="stroke-foreground"
      strokeWidth={1.25}
    >
      <circle cx="8" cy="8" r="5.5" />
      <circle cx="24" cy="8" r="5.5" />
      <line x1="13.5" y1="8" x2="18.5" y2="8" />
    </svg>
  );
}

export default Sidebar;
