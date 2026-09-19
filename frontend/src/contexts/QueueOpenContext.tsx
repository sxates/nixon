'use client';

import { createContext, useContext, useState, type ReactNode } from 'react';

export interface QueueOpenValue {
  open: boolean;
  setOpen: (open: boolean) => void;
}

const QueueOpenContext = createContext<QueueOpenValue | null>(null);

/**
 * Whether the ONE queue popover (the sidebar's `QueueRow`, specs/0064 W5) is open — shared so
 * that Today's "Process meetings" button can open that same popover instead of keeping a
 * second, duplicate queue surface of its own (specs/0063 W3 Task 6).
 */
export function QueueOpenProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return <QueueOpenContext.Provider value={{ open, setOpen }}>{children}</QueueOpenContext.Provider>;
}

export function useQueueOpen(): QueueOpenValue {
  const ctx = useContext(QueueOpenContext);
  if (!ctx) throw new Error('useQueueOpen must be used within a QueueOpenProvider');
  return ctx;
}
