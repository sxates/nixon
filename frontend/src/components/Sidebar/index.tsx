'use client';

import React, { useEffect } from 'react';
import {
  CalendarDays,
  CheckSquare,
  FileAudio,
  Home,
  Menu,
  MessageSquare,
  Settings,
  Users,
  type LucideIcon,
} from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { DeckIcon } from '@/components/ui/deck-icon';
import { cn } from '@/lib/utils';
import { QueueRow } from './QueueRow';
import { SIDEBAR_ICON_SLOT, SIDEBAR_ROW } from './row';
import { UpdateRow } from './UpdateRow';

import DevBadge from '../DevBadge';

/** One entry in the primary navigation. */
interface NavItem {
  label: string;
  path: string;
  /** The silkscreened glyph for this destination (specs/0057 §2). */
  icon: LucideIcon;
  /** Returns true when this item should render as active for the current path. */
  isActive: (pathname: string | null) => boolean;
}

const NAV_ITEMS: NavItem[] = [
  {
    label: 'Today',
    path: '/',
    icon: Home,
    isActive: (p) => p === '/',
  },
  {
    label: 'All meetings',
    // The meeting-details view is the destination of every meeting row, so it
    // belongs to the "All meetings" section for active-state purposes.
    path: '/meetings',
    icon: CalendarDays,
    isActive: (p) => (p?.startsWith('/meetings') ?? false) || (p?.startsWith('/meeting-details') ?? false),
  },
  {
    // Cross-meeting task hub (specs/0034).
    label: 'Action items',
    path: '/tasks',
    icon: CheckSquare,
    isActive: (p) => p?.startsWith('/tasks') ?? false,
  },
  {
    label: 'People',
    path: '/people',
    icon: Users,
    isActive: (p) => p?.startsWith('/people') ?? false,
  },
  {
    // Cross-meeting Ask-AI (specs/0035).
    label: 'Ask AI',
    path: '/ask',
    icon: MessageSquare,
    isActive: (p) => p?.startsWith('/ask') ?? false,
  },
];

/** The brushed-panel surface shared by both sidebar branches (specs/0057 decision 5). */
const PANEL_SURFACE =
  'bg-panel shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_-1px_0_0_hsl(var(--bevel-lo))]';

/** The amber index bar that marks the active row; transparent when idle. */
const INDEX_BAR_ON = 'bg-brand shadow-[0_0_6px_-1px_hsl(var(--brand)/0.6)]';

const Sidebar: React.FC = () => {
  const router = useRouter();
  const pathname = usePathname();
  const { isCollapsed, toggleCollapse } = useSidebar();

  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();

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

  const importEnabled = betaFeatures.importAndRetranscribe;

  // ----- Collapsed: the icon rail ------------------------------------------
  if (isCollapsed) {
    return (
      <div className="fixed top-0 left-0 z-40 h-screen">
        <div
          className={cn(
            'relative flex h-screen w-16 flex-col items-center border-r border-border transition-all duration-300',
            PANEL_SURFACE,
          )}
        >
          <WalnutCheek />
          <HamburgerButton expanded={false} onToggle={toggleCollapse} className="mt-4" />
          <div className="mt-3">
            <NixonMark />
          </div>

          <nav className="mt-4 flex w-full flex-1 flex-col items-center gap-0.5 overflow-y-auto">
            {NAV_ITEMS.map((item) => {
              const active = item.isActive(pathname);
              return (
                <button
                  key={item.label}
                  onClick={() => router.push(item.path)}
                  aria-label={item.label}
                  title={item.label}
                  aria-current={active ? 'page' : undefined}
                  className="relative flex h-10 w-full items-center justify-center transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <IndexBar active={active} className="left-1.5" />
                  <DeckIcon
                    icon={item.icon}
                    size={16}
                    className={active ? 'text-foreground' : 'text-engrave'}
                  />
                </button>
              );
            })}
          </nav>

          <UpdateRow collapsed />

          <button
            onClick={() => router.push('/settings')}
            aria-label="Settings"
            title="Settings"
            className="flex h-10 w-full items-center justify-center text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <DeckIcon icon={Settings} size={16} />
          </button>

          {/* Background work (specs/0064 W5) — below Settings, the lamp alone when collapsed. */}
          <QueueRow collapsed />

          <div className="mb-3">
            <DevBadge isCollapsed />
          </div>
        </div>
      </div>
    );
  }

  // ----- Expanded sidebar ---------------------------------------------------
  return (
    <div className="fixed top-0 left-0 z-40 h-screen">
      <div
        className={cn(
          'relative flex h-screen w-64 flex-col border-r border-border transition-all duration-300',
          PANEL_SURFACE,
        )}
      >
        <WalnutCheek />

        {/* Top bar: hamburger toggle + engraved wordmark + DEV badge. Shares the row
            geometry so the hamburger sits in the same icon column as every glyph below it
            and NIXON starts where the nav labels do. */}
        <div className={cn(SIDEBAR_ROW, 'flex-shrink-0 pb-3 pt-4')}>
          <HamburgerButton expanded onToggle={toggleCollapse} />
          <span className="u-section-label text-[13px] tracking-[0.18em] text-foreground">
            NIXON
          </span>
          <div className="ml-auto">
            <DevBadge />
          </div>
        </div>

        {/* Primary navigation — index bar + glyph + engraved label. */}
        <nav className="mt-2 flex-1 overflow-y-auto">
          <div className="flex flex-col">
            {NAV_ITEMS.map((item) => {
              const active = item.isActive(pathname);
              return (
                <button
                  key={item.label}
                  onClick={() => router.push(item.path)}
                  aria-current={active ? 'page' : undefined}
                  className={cn(SIDEBAR_ROW, 'relative h-8 transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
                >
                  <IndexBar active={active} className="left-2" />
                  <span className={SIDEBAR_ICON_SLOT}>
                    <DeckIcon
                      icon={item.icon}
                      className={active ? 'text-foreground' : 'text-engrave'}
                    />
                  </span>
                  <span
                    className={cn('u-section-label', active ? 'text-foreground' : 'text-engrave')}
                  >
                    {item.label}
                  </span>
                </button>
              );
            })}
          </div>

          {/* Import audio (beta) — secondary action, gated by the beta flag */}
          {importEnabled && (
            <button
              onClick={() => openImportDialog()}
              className={cn(SIDEBAR_ROW, 'relative mt-2 h-8 w-full text-brand transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
            >
              <span className={SIDEBAR_ICON_SLOT}>
                <DeckIcon icon={FileAudio} />
              </span>
              <span className="u-section-label text-brand">Import audio</span>
              <span className="u-section-label ml-auto text-[9px] text-muted-foreground">Beta</span>
            </button>
          )}
        </nav>

        {/* Footer — settings, updates, queue (the avatar puck is gone, specs/0057 Plan 3).
            Vertical padding only: a horizontal inset here would push these rows 8px right of
            the nav rows above, which is exactly the misalignment SIDEBAR_ROW exists to stop. */}
        <div className="flex-shrink-0 border-t border-border py-2">
          <UpdateRow />

          <button
            onClick={() => router.push('/settings')}
            className={cn(SIDEBAR_ROW, 'relative h-8 w-full text-left text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
            aria-label="Settings"
          >
            <span className={SIDEBAR_ICON_SLOT}>
              <DeckIcon icon={Settings} />
            </span>
            <span className="u-section-label">Settings</span>
          </button>

          {/* Background work (specs/0064 W5) — the queue moved off the transport rail, which
              is the recording surface; this is where app state lives. */}
          <QueueRow />
        </div>
      </div>
    </div>
  );
};

/** The walnut cheek running down the machine's left edge (specs/0057 decision 5). */
function WalnutCheek() {
  return <span aria-hidden className="absolute inset-y-0 left-0 z-10 w-1.5 bg-walnut" />;
}

/** The 1.5px amber bar that indexes the active nav row. */
function IndexBar({ active, className }: { active: boolean; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        'absolute top-2 bottom-2 w-[1.5px] [transition-property:background-color] [transition-duration:120ms]',
        active ? INDEX_BAR_ON : 'bg-transparent',
        className,
      )}
    />
  );
}

/** The ⊙—⊙ reel mark: the wordmark's stand-in on the collapsed rail. */
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

/** The single sidebar control: a hamburger that expands/collapses the bar. */
function HamburgerButton({
  expanded,
  onToggle,
  className = '',
}: {
  expanded: boolean;
  onToggle: () => void;
  className?: string;
}) {
  return (
    <button
      onClick={onToggle}
      aria-label={expanded ? 'Collapse sidebar' : 'Expand sidebar'}
      aria-expanded={expanded}
      title={expanded ? 'Collapse sidebar' : 'Expand sidebar'}
      className={cn(
        // -m-2/p-2: the glyph stays in the shared icon column while the clickable area
        // grows back to a comfortable size around it (owner feedback 2026-09-19).
        'flex flex-shrink-0 items-center justify-center rounded-[3px] text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        expanded ? '-m-2 p-2' : 'h-9 w-9',
        className,
      )}
    >
      {/* Expanded: the glyph sits in the shared 14px icon column so it lines up with every
          row below. Collapsed: the rail centres a slightly larger mark, as before. */}
      {expanded ? (
        <span className={SIDEBAR_ICON_SLOT}>
          <DeckIcon icon={Menu} />
        </span>
      ) : (
        <DeckIcon icon={Menu} size={18} />
      )}
    </button>
  );
}

export default Sidebar;
