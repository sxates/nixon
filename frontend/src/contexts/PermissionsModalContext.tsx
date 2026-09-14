'use client';

import { createContext, useContext, useState, useCallback, ReactNode } from 'react';

interface PermissionsModalContextType {
  isOpen: boolean;
  openPermissionsModal: () => void;
  closePermissionsModal: () => void;
  setOpen: (open: boolean) => void;
}

const PermissionsModalContext = createContext<PermissionsModalContextType | null>(null);

/**
 * Access the global "Enable recording" permissions modal. Used by both the
 * expanded and collapsed sidebar Permissions nav items so a single modal
 * instance (mounted in the layout) is shared across the app.
 */
export const usePermissionsModal = () => {
  const ctx = useContext(PermissionsModalContext);
  if (!ctx) throw new Error('usePermissionsModal must be used within a PermissionsModalProvider');
  return ctx;
};

export function PermissionsModalProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);

  const openPermissionsModal = useCallback(() => setIsOpen(true), []);
  const closePermissionsModal = useCallback(() => setIsOpen(false), []);

  return (
    <PermissionsModalContext.Provider
      value={{ isOpen, openPermissionsModal, closePermissionsModal, setOpen: setIsOpen }}
    >
      {children}
    </PermissionsModalContext.Provider>
  );
}
