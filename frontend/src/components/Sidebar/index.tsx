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
  SquarePen,
  Users,
  type LucideIcon,
} from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { DeckIcon } from '@/components/ui/deck-icon';
import { cn } from '@/lib/utils';

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
    label: 'Home',
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
  const { isCollapsed, toggleCollapse, handleRecordingToggle, handleNewNote } = useSidebar();

  // Recording state from the single source of truth.
  const { isRecording } = useRecordingState();
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

          <button
            onClick={() => router.push('/settings')}
            aria-label="Settings"
            title="Settings"
            className="mb-2 flex h-10 w-full items-center justify-center text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <DeckIcon icon={Settings} size={16} />
          </button>

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

        {/* Top bar: hamburger toggle + engraved wordmark + DEV badge */}
        <div className="flex flex-shrink-0 items-center gap-2 px-3 pb-3 pt-4">
          <HamburgerButton expanded onToggle={toggleCollapse} />
          <span className="u-section-label text-[13px] tracking-[0.18em] text-foreground">
            NIXON
          </span>
          <div className="ml-auto">
            <DevBadge />
          </div>
        </div>

        {/* New recording — reuses the existing recording-start handler */}
        <div className="flex-shrink-0 px-3">
          <button
            onClick={handleRecordingToggle}
            disabled={isRecording}
            className={cn(
              'flex w-full items-center gap-2.5 rounded-[3px] border border-border bg-key px-3 py-2 text-sm transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              'shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),0_1px_0_hsl(var(--bevel-lo))]',
              isRecording ? 'cursor-not-allowed' : 'hover:bg-well',
            )}
          >
            <span
              className={cn(
                'h-2 w-2 flex-shrink-0 rounded-full bg-record',
                isRecording && 'animate-pulse',
              )}
              aria-hidden="true"
            />
            <span className="font-semibold text-foreground">
              {isRecording ? 'Recording…' : 'New recording'}
            </span>
            {!isRecording && (
              <kbd className="ml-auto font-mono text-[11px] text-muted-foreground">⌘N</kbd>
            )}
          </button>

          {/* New note — creates a notes-only meeting (no recording) and opens it. */}
          <button
            onClick={() => void handleNewNote()}
            className="mt-1.5 flex w-full items-center gap-2.5 rounded-[3px] px-3 py-2 text-sm text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <DeckIcon icon={SquarePen} className="flex-shrink-0" />
            <span>New note</span>
          </button>
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
                  className="relative flex h-8 items-center gap-2.5 pl-5 pr-3.5 transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <IndexBar active={active} className="left-2" />
                  <DeckIcon
                    icon={item.icon}
                    className={active ? 'text-foreground' : 'text-engrave'}
                  />
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
              className="relative mt-2 flex h-8 w-full items-center gap-2.5 pl-5 pr-3.5 text-brand transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <DeckIcon icon={FileAudio} className="flex-shrink-0" />
              <span className="u-section-label text-brand">Import audio</span>
              <span className="u-section-label ml-auto text-[9px] text-muted-foreground">Beta</span>
            </button>
          )}
        </nav>

        {/* Footer — settings entry (the avatar puck is gone, specs/0057 Plan 3) */}
        <div className="flex-shrink-0 border-t border-border p-2">
          <button
            onClick={() => router.push('/settings')}
            className="relative flex h-8 w-full items-center gap-2.5 pl-3 pr-3.5 text-left text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Settings"
          >
            <DeckIcon icon={Settings} className="flex-shrink-0" />
            <span className="u-section-label">Settings</span>
          </button>
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
        'flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-[3px] text-engrave transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        className,
      )}
    >
      <DeckIcon icon={Menu} size={18} />
    </button>
  );
}

export default Sidebar;
