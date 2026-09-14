"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Loader2 } from 'lucide-react';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  onOpenFolder: () => Promise<void>;
  hasSummary: boolean;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  onOpenFolder: _onOpenFolder,
  hasSummary
}: SummaryUpdaterButtonGroupProps) {
  return (
    <ButtonGroup>
      {/* Save button */}
      <Button
        variant="outline"
        size="xs"
        className={`${isDirty ? 'bg-brand/20' : ""}`}
        title={isSaving ? "Saving" : "Save Changes"}
        onClick={() => {
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="mr-2 animate-spin" size={16} />
            <span>Saving…</span>
          </>
        ) : (
          <span>Save</span>
        )}
      </Button>

      {/* Copy button */}
      <Button
        variant="outline"
        size="xs"
        title="Copy Summary"
        onClick={() => {
          onCopy();
        }}
        disabled={!hasSummary}
        className="cursor-pointer"
      >
        <span>Copy</span>
      </Button>
    </ButtonGroup>
  );
}
