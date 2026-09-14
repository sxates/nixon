import React from 'react';
import type { LucideIcon } from 'lucide-react';

interface DeckIconProps {
  /** The lucide glyph to draw. */
  icon: LucideIcon;
  className?: string;
  /** Pixel size of the square glyph box. Deck chrome uses 14px. */
  size?: number;
}

/**
 * The one way icons are drawn in the Nixon deck chrome (specs/0057 §2): thin,
 * square-cut strokes with mitered joins so a glyph reads as silkscreen on a
 * brushed panel rather than as a rounded web icon. Always decorative — the
 * accessible name comes from the control that wraps it.
 */
export function DeckIcon({ icon: Icon, className, size = 14 }: DeckIconProps) {
  return (
    <Icon
      size={size}
      strokeWidth={1.5}
      strokeLinecap="square"
      strokeLinejoin="miter"
      className={className}
      aria-hidden
    />
  );
}

export default DeckIcon;
