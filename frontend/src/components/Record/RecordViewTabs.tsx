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
}: {
  view: RecordView;
  onViewChange: (view: RecordView) => void;
}) {
  return (
    <Tabs value={view} onValueChange={(v) => isRecordView(v) && onViewChange(v)}>
      <TabsList aria-label="Recording view">
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
