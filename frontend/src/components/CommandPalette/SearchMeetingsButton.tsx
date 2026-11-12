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

import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { Search } from '@/components/ui/icons';
import { openCommandPalette } from '@/components/CommandPalette';

/** The 32px bordered key beside the header's other icon buttons. */
export function SearchMeetingsButton({ className }: { className?: string }) {
  return (
    <IconButton
      size="md"
      label="Search meetings (⌘K)"
      aria-keyshortcuts="Meta+K"
      onClick={openCommandPalette}
      icon={<DeckIcon icon={Search} size={14} />}
      className={className}
    />
  );
}
