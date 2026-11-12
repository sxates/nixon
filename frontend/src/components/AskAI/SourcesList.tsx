'use client';

/**
 * SourcesList (specs/0038 #4) — the "Sources" block under an Ask-AI answer.
 *
 * One implementation shared by the /ask page and the /saved-question sub-page so the
 * cited/uncited rendering (and its egress-accountability "Searched but not cited" section) can't
 * drift between them. Each row links to its meeting. Renders nothing when there are no sources.
 */

import { useRouter } from 'next/navigation';
import { LinkRow } from '@/components/ui/list-row';
import type { SourceMeeting } from '@/lib/ask-ai';
import { formatMeetingDate } from '@/lib/format-date';

function SourceRow({ source, muted }: { source: SourceMeeting; muted: boolean }) {
  const router = useRouter();
  return (
    <LinkRow
      title={source.title?.trim() || 'Untitled meeting'}
      meta={formatMeetingDate(source.createdAt) || undefined}
      muted={muted}
      hint="Open this meeting"
      onClick={() => router.push(`/meeting-details?id=${source.meetingId}`)}
    />
  );
}

export function SourcesList({ sources }: { sources: SourceMeeting[] }) {
  if (sources.length === 0) return null;
  const cited = sources.filter((s) => s.cited);
  const uncited = sources.filter((s) => !s.cited);

  return (
    <div className="mt-6 border-t border-border pt-4">
      <h2 className="u-section-label">Sources</h2>
      {cited.length > 0 && (
        <div className="mt-2 flex flex-col gap-0.5">
          {cited.map((s) => (
            <SourceRow key={s.meetingId} source={s} muted={false} />
          ))}
        </div>
      )}
      {uncited.length > 0 && (
        <>
          {/* Egress accountability: everything sent is listed, even when uncited. */}
          <p className="u-meta mt-3 px-2">Searched but not cited</p>
          <div className="mt-1 flex flex-col gap-0.5">
            {uncited.map((s) => (
              <SourceRow key={s.meetingId} source={s} muted />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
