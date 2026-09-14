'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';

/**
 * Notes now live in the meeting-details "My Notes" tab (spec 0003 pivot). This component
 * redirects the legacy `/notes/[id]` route to the meeting's details page.
 *
 * Under static export the `[id]` segment isn't pre-rendered for runtime ids, so the real id
 * is read client-side from the pathname (falling back to the build-time param).
 */
export default function NotesRedirect({ paramId }: { paramId: string }) {
  const router = useRouter();

  useEffect(() => {
    let id = paramId;
    if (typeof window !== 'undefined') {
      const segments = window.location.pathname.split('/').filter(Boolean);
      const last = segments[segments.length - 1];
      if (last) id = decodeURIComponent(last);
    }
    router.replace(`/meeting-details?id=${encodeURIComponent(id)}`);
  }, [paramId, router]);

  return null;
}
