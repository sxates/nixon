'use client';

/**
 * The visible entry point to ⌘K search (specs/0054 W3).
 *
 * Full-text search across transcripts, summaries and notes has existed since 1.4,
 * but for a long time ⌘K was its only affordance, so it went unfound. It lives in
 * one component because it now appears in more than one header and the two had
 * already drifted apart once — same icon, different size, different behaviour at
 * narrow widths.
 */

import { Search } from 'lucide-react';
import { openCommandPalette } from '@/components/CommandPalette';
import { cn } from '@/lib/utils';

export function SearchMeetingsButton({ className }: { className?: string }) {
  return (
    <button
      type="button"
      onClick={openCommandPalette}
      aria-label="Search meetings (⌘K)"
      aria-keyshortcuts="Meta+K"
      title="Search meetings (⌘K)"
      className={cn(
        'inline-flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-[3px] border border-border bg-card text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        className,
      )}
    >
      {/* 12px glyph in a 32px box — 10px of breathing room a side. */}
      <Search className="h-3 w-3" aria-hidden="true" />
    </button>
  );
}
