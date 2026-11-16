'use client';

/**
 * The drag handle for a pointer-reordered list (`useDragReorder`): a narrow grip in the
 * row's gutter, not a 28px key. It shows while its row (the nearest `group`) is hovered,
 * while it has focus, and while its row is being dragged. Shared by the To-do list and the
 * Topics page (specs/0088, 0090).
 */

import type { ComponentProps } from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { GripVertical } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

type GripHandlers = Omit<ComponentProps<typeof IconButton>, 'label' | 'icon' | 'variant' | 'className'>;

export function ReorderGrip({ dragging, ...handlers }: GripHandlers & { dragging: boolean }) {
  return (
    <IconButton
      variant="ghost"
      label="Drag to reorder"
      {...handlers}
      icon={<DeckIcon icon={GripVertical} size={14} />}
      className={cn(
        'h-[18px] w-4 cursor-grab touch-none text-muted-foreground/40 opacity-0 transition-opacity hover:bg-transparent hover:text-muted-foreground focus-visible:opacity-100 group-hover:opacity-100 active:cursor-grabbing',
        dragging && 'opacity-100',
      )}
    />
  );
}
