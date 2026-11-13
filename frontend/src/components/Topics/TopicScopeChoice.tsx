'use client';

/**
 * The "which meetings" choice shared by New topic and Add to topic… (specs/0089): This
 * meeting / Every meeting in this series, plus the 90-day backfill box for a series.
 */

import { Checkbox } from '@/components/ui/checkbox';
import { SegmentedControl } from '@/components/ui/segmented-control';

export type TopicScope = 'meeting' | 'series';

export interface TopicScopeChoiceProps {
  scope: TopicScope;
  onScopeChange: (scope: TopicScope) => void;
  backfill: boolean;
  onBackfillChange: (backfill: boolean) => void;
  /** False greys out the series option (no series key and no title to match on). */
  seriesAvailable?: boolean;
}

export function TopicScopeChoice({
  scope,
  onScopeChange,
  backfill,
  onBackfillChange,
  seriesAvailable = true,
}: TopicScopeChoiceProps) {
  return (
    <div className="flex flex-col gap-2">
      <SegmentedControl
        aria-label="Which meetings"
        size="lg"
        value={scope}
        onChange={(v) => onScopeChange(v as TopicScope)}
        options={[
          { value: 'meeting', label: 'This meeting' },
          { value: 'series', label: 'Every meeting in this series', disabled: !seriesAvailable },
        ]}
      />
      {scope === 'series' ? (
        <Checkbox
          checked={backfill}
          onCheckedChange={onBackfillChange}
          label="Also add this series' meetings from the last 90 days"
        />
      ) : null}
    </div>
  );
}
