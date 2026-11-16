'use client';

import { useCallback } from 'react';
import { useRouter } from 'next/navigation';
import { canGoBackInApp } from '@/lib/nav-history';

/**
 * The one Back behaviour (every page header's back arrow, every "Back" button): return to
 * the page the user came from, or to `fallbackHref` when there is no in-app page behind
 * this one (a cold deep link, the first page after launch). See `lib/nav-history.ts`.
 */
export function useBack(fallbackHref: string): () => void {
  const router = useRouter();
  return useCallback(() => {
    if (canGoBackInApp()) router.back();
    else router.push(fallbackHref);
  }, [router, fallbackHref]);
}
