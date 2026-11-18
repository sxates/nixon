'use client';

import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { isRecordView, type RecordView } from '@/lib/record-view';

/**
 * Transcript / Focus: which view the record screen shows (owner feedback, v0.18). Underline
 * `Tabs`, because it changes the view rather than filtering it (docs/design-system.md). The
 * panes live elsewhere and stay mounted, so the triggers name no panel.
 */
export function RecordViewTabs({
  view,
  onViewChange,
  className,
}: {
  view: RecordView;
  onViewChange: (view: RecordView) => void;
  /** On the tab list (the header drops its rule: the band's own bottom rule is under it). */
  className?: string;
}) {
  return (
    <Tabs value={view} onValueChange={(v) => isRecordView(v) && onViewChange(v)}>
      <TabsList aria-label="Recording view" className={className}>
        <TabsTrigger value="transcript" aria-controls={undefined}>
          Transcript
        </TabsTrigger>
        <TabsTrigger value="focus" aria-controls={undefined}>
          Focus
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );
}
