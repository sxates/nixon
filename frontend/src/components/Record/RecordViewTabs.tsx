'use client';

import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { isRecordView, type RecordView } from '@/lib/record-view';

/**
 * Transcript · My Notes · Agenda: which view the record screen shows (owner feedback, v0.18).
 * Underline `Tabs`, because it changes the view rather than filtering it
 * (docs/design-system.md). The panes live elsewhere and stay mounted, so the triggers name
 * no panel. Agenda needs the meeting row (its prep lives on it), so it is left out until the
 * row exists, the same rule as the Transcript rail's Agenda tab.
 */
export function RecordViewTabs({
  view,
  onViewChange,
  agendaAvailable = true,
  className,
}: {
  view: RecordView;
  onViewChange: (view: RecordView) => void;
  agendaAvailable?: boolean;
  /** On the tab list (the header drops its rule: the band's own bottom rule is under it). */
  className?: string;
}) {
  return (
    <Tabs value={view} onValueChange={(v) => isRecordView(v) && onViewChange(v)}>
      <TabsList aria-label="Recording view" className={className}>
        <TabsTrigger value="transcript" aria-controls={undefined}>
          Transcript
        </TabsTrigger>
        <TabsTrigger value="notes" aria-controls={undefined}>
          My Notes
        </TabsTrigger>
        {agendaAvailable && (
          <TabsTrigger value="agenda" aria-controls={undefined}>
            Agenda
          </TabsTrigger>
        )}
      </TabsList>
    </Tabs>
  );
}
