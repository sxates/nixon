'use client';

import { useEffect } from 'react';
import { installNavHistory } from '@/lib/nav-history';

/** Mounted once in the root layout: starts the in-app history that `useBack` reads. */
export function NavHistoryTracker() {
  useEffect(() => installNavHistory(), []);
  return null;
}
