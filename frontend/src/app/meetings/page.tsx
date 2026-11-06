'use client';

/**
 * `/meetings` was the All meetings page; it is now the Meetings page's List (or Month)
 * view at `/`. Kept as a redirect so old links and the back stack still land somewhere:
 * the query is carried over, and `?view=` defaults to the List it used to open on. A
 * static export has no server redirects, so this is a client-side `replace`.
 */

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';
import { meetingsRedirectTarget } from '@/lib/meetings-view';

export default function MeetingsRedirect() {
  const router = useRouter();
  useEffect(() => {
    router.replace(meetingsRedirectTarget(window.location.search));
  }, [router]);
  return <div className="h-page bg-background" />;
}
