'use client';

/**
 * MenuButton — the overflow `…` trigger (docs/design-system.md §Component catalog). An
 * `IconButton` preset: ghost, 28px, the 14px horizontal ellipsis drawn through `DeckIcon`.
 * Put it inside a `DropdownMenuTrigger asChild`; it forwards its ref and every prop the
 * trigger injects (`aria-expanded`, `data-state`, the pointer handlers) to the button.
 *
 * The label defaults to "More actions"; pass something more specific ("Meeting options")
 * when the screen has more than one overflow menu.
 */

import { forwardRef } from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton, type IconButtonProps } from '@/components/ui/icon-button';
import { MoreHorizontal } from '@/components/ui/icons';

export interface MenuButtonProps extends Omit<IconButtonProps, 'icon' | 'label'> {
  label?: string;
}

export const MenuButton = forwardRef<HTMLButtonElement, MenuButtonProps>(function MenuButton(
  { label = 'More actions', variant = 'ghost', ...rest },
  ref,
) {
  return (
    <IconButton
      ref={ref}
      label={label}
      variant={variant}
      icon={<DeckIcon icon={MoreHorizontal} size={14} />}
      {...rest}
    />
  );
});
