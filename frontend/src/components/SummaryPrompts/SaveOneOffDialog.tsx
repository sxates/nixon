'use client';

import { useEffect, useState } from 'react';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { MAX_PROMPT_NAME_CHARS } from '@/lib/summary-prompts';

interface SaveOneOffDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Only series meetings can reuse the prompt for future occurrences. */
  hasSeries: boolean;
  /** Suggested library name. */
  defaultName: string;
  /** Persist the choice. A rejection keeps the dialog open (the caller toasts). */
  onSave: (opts: { name?: string; toLibrary: boolean; toSeries: boolean }) => Promise<void>;
  /** "Not now" (also Escape / close). */
  onSkip: () => void;
}

/**
 * Offered right after a one-off prompt is set (specs/0079 W4): keep it for future
 * meetings in the series and/or in the prompt library, or just use it this once.
 */
export function SaveOneOffDialog({
  open,
  onOpenChange,
  hasSeries,
  defaultName,
  onSave,
  onSkip,
}: SaveOneOffDialogProps) {
  const [toSeries, setToSeries] = useState(false);
  const [toLibrary, setToLibrary] = useState(false);
  const [name, setName] = useState(defaultName);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setToSeries(false);
    setToLibrary(false);
    setName(defaultName);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-seed on open
  }, [open]);

  const canSave = !isSaving && (toSeries || toLibrary) && (!toLibrary || name.trim().length > 0);

  const handleSave = async () => {
    if (!canSave) return;
    setIsSaving(true);
    try {
      await onSave({
        name: toLibrary ? name.trim() : undefined,
        toLibrary,
        toSeries: hasSeries && toSeries,
      });
    } catch {
      // The hook already toasted; stay open.
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (next) onOpenChange(true);
        else onSkip();
      }}
      title="Keep this prompt?"
      body="It is set for this meeting. You can also keep it for later."
      tone="brand"
      confirmLabel="Save"
      cancelLabel="Not now"
      busy={isSaving}
      confirmDisabled={!canSave}
      onConfirm={handleSave}
    >
      <div className="space-y-3">
        {hasSeries && (
          <div className="flex items-center gap-3">
            <Switch id="save-one-off-series" checked={toSeries} onCheckedChange={setToSeries} />
            <Label htmlFor="save-one-off-series">Use for future meetings in this series</Label>
          </div>
        )}
        <div className="flex items-center gap-3">
          <Switch id="save-one-off-library" checked={toLibrary} onCheckedChange={setToLibrary} />
          <Label htmlFor="save-one-off-library">Save to my prompt library</Label>
        </div>
        {toLibrary && (
          <div className="space-y-1.5 pl-12">
            <Label htmlFor="save-one-off-name">Prompt name</Label>
            <Input
              id="save-one-off-name"
              value={name}
              maxLength={MAX_PROMPT_NAME_CHARS}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
        )}
      </div>
    </ConfirmDialog>
  );
}
