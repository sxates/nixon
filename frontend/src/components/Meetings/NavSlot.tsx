'use client';

import { createContext, useContext, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * The right-hand end of the Meetings view-tab row. Each view's own date navigation
 * ("< Today >", "This week", "< November 2026 > Today") renders into it, so the tabs and the
 * navigation share one row and the content gets the vertical space back. The navigation
 * keeps its state where it lives; only the DOM position moves.
 */
const NavSlotContext = createContext<HTMLElement | null>(null);

export const NavSlotProvider = NavSlotContext.Provider;

/** Renders `children` into the tab row's slot when one is mounted, otherwise in place. */
export function NavSlotPortal({ children }: { children: ReactNode }) {
  const slot = useContext(NavSlotContext);
  return slot ? createPortal(children, slot) : <>{children}</>;
}

/** True when a slot is mounted (the caller can drop its own row chrome). */
export function useHasNavSlot(): boolean {
  return useContext(NavSlotContext) !== null;
}
