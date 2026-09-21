'use client';

import { useRouter } from 'next/navigation';
import { Plus, Sparkles } from 'lucide-react';
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
  onAddMeeting: () => void;
}

/** Today header — greeting + day summary, ⌘K search, Ask AI, Add meeting. */
export function TodayHeader({ now, daySummary, onAddMeeting }: TodayHeaderProps) {
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
          <Button variant="brand" onClick={onAddMeeting} className="gap-2">
            <Plus className="h-[13px] w-[13px]" aria-hidden="true" />
            Add meeting
          </Button>
        </>
      }
    />
  );
}
