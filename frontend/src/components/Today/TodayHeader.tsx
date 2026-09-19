'use client';

import { useRouter } from 'next/navigation';
import { Sparkles } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { PageHeader } from '@/components/ui/page-header';

/** Time-of-day greeting from the local hour. */
function greeting(now: Date): string {
  const h = now.getHours();
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

interface TodayHeaderProps {
  now: Date;
  /** Summary line under the greeting — reflects the day (or week) in view. */
  daySummary: string;
  isRecording: boolean;
  onNewNote: () => void;
  onRecord: () => void;
}

/** Today header — greeting + day summary, ⌘K search, Ask AI, New note, Record. */
export function TodayHeader({ now, daySummary, isRecording, onNewNote, onRecord }: TodayHeaderProps) {
  const router = useRouter();
  return (
    <PageHeader
      title={greeting(now)}
      subtitle={daySummary}
      actions={
        <>
          <SearchMeetingsButton />
          <button
            type="button"
            onClick={() => router.push('/ask')}
            aria-label="Ask AI about your meetings"
            className="hidden items-center gap-[7px] rounded-[3px] border border-border bg-card px-[11px] py-[7px] text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:flex"
          >
            <Sparkles className="h-[13px] w-[13px]" aria-hidden="true" />
            <span>Ask AI</span>
          </button>
          <ProcessMeetingsButton />
          <Button variant="outline" onClick={onNewNote} className="gap-2">
            <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/40" aria-hidden="true" />
            New note
          </Button>
          <Button variant="brand" onClick={onRecord} className="gap-2" disabled={isRecording}>
            <span className="h-2 w-2 rounded-full bg-brand-foreground" aria-hidden="true" />
            {isRecording ? 'Recording…' : 'Record'}
            {!isRecording && (
              <kbd className="ml-1 rounded bg-brand-foreground/20 px-1.5 py-0.5 font-mono text-[10px] font-medium tracking-wide">
                ⌘⇧R
              </kbd>
            )}
          </Button>
        </>
      }
    />
  );
}
