'use client';

import { useState, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { SettingsRow } from '@/components/ui/settings';
import { RetentionChangeDialog } from '@/components/RetentionChangeDialog';
import { patchRecordingPreferences } from '@/lib/recording-preferences';
import { errorText } from '@/lib/recordings-move';
import {
  applyRetentionChoice,
  isLoweringRetention,
  retentionChoiceDescription,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
  retentionPolicyFromChoice,
  retentionReportToast,
  retentionRuleSentence,
  type AudioRetentionChoice,
  type RetentionPreview,
  type RetentionReport,
} from '@/lib/audio-retention';
import type { RecordingPreferences } from './RecordingSettings';

const PRESET_DAYS = [7, 30, 90];

interface AudioRetentionRowProps {
  preferences: RecordingPreferences;
  setPreferences: Dispatch<SetStateAction<RecordingPreferences>>;
  onSave?: (preferences: RecordingPreferences) => void;
  disabled?: boolean;
}

/**
 * "Delete audio recordings" (specs/0072 W3).
 *
 * Shortening how long audio is kept asks first when it would delete anything now: a dry
 * run (`api_preview_audio_retention`) counts the meetings and bytes, RetentionChangeDialog
 * shows them, and only "Delete audio" saves the setting and runs the sweep at once
 * (`api_apply_audio_retention_now`), then toasts what the sweep REPORTED. Cancel saves
 * nothing. Keeping audio longer, or a change that deletes nothing, just saves.
 */
export function AudioRetentionRow({
  preferences,
  setPreferences,
  onSave,
  disabled = false,
}: AudioRetentionRowProps) {
  const saved = retentionChoiceFromPreferences(preferences);
  const [pending, setPending] = useState<{
    choice: AudioRetentionChoice;
    preview: RetentionPreview;
  } | null>(null);
  const [checking, setChecking] = useState(false);
  const shown = pending?.choice ?? saved;

  // Writes only the retention fields (read-modify-write), so a stale copy of the other
  // preferences can't overwrite another tab's change.
  const save = async (choice: AudioRetentionChoice) => {
    const next = applyRetentionChoice(preferences, choice);
    const patch = {
      audio_retention: next.audio_retention,
      auto_save: next.auto_save,
      retention_days: next.retention_days,
    };
    await patchRecordingPreferences<RecordingPreferences>(patch);
    setPreferences((prev) => ({ ...prev, ...patch }));
    onSave?.(next);
  };

  const saveAndSayRule = async (choice: AudioRetentionChoice) => {
    try {
      await save(choice);
      toast.success('Preference saved', { description: retentionRuleSentence(choice) });
    } catch (error) {
      console.error('Failed to save audio retention preference:', error);
      toast.error('Failed to save preference');
    }
  };

  const handleChange = async (value: string) => {
    const choice = retentionChoiceFromSelectValue(value);
    if (choice === saved) return;
    if (!isLoweringRetention(saved, choice)) {
      await saveAndSayRule(choice);
      return;
    }
    setChecking(true);
    let preview: RetentionPreview;
    try {
      preview = await invoke<RetentionPreview>('api_preview_audio_retention', {
        policy: retentionPolicyFromChoice(choice),
      });
    } catch (error) {
      // Without the count we can't ask honestly, so nothing is saved.
      console.error('Failed to preview audio retention:', error);
      toast.error("Couldn't check which audio would be deleted", {
        description: errorText(error),
      });
      return;
    } finally {
      setChecking(false);
    }
    if (preview.meetings > 0) {
      setPending({ choice, preview });
      return;
    }
    await saveAndSayRule(choice);
  };

  const handleConfirm = async () => {
    if (!pending) return;
    try {
      await save(pending.choice);
    } catch (error) {
      console.error('Failed to save audio retention preference:', error);
      toast.error('Failed to save preference');
      setPending(null);
      return;
    }
    try {
      const report = await invoke<RetentionReport>('api_apply_audio_retention_now');
      const { title, description } = retentionReportToast(report);
      toast.success(title, description ? { description } : undefined);
    } catch (error) {
      console.error('Failed to apply audio retention:', error);
      toast.error("Saved, but the audio couldn't be deleted yet", {
        description: 'Nixon tries again within the hour.',
      });
    }
    setPending(null);
  };

  return (
    <>
      <SettingsRow
        label="Delete audio recordings"
        htmlFor="audio-retention"
        description={retentionChoiceDescription(shown)}
        control={
          <Select
            value={retentionChoiceToSelectValue(shown)}
            onValueChange={(value) => void handleChange(value)}
            disabled={disabled || checking || pending !== null}
          >
            <SelectTrigger id="audio-retention" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="once-processed">Once processed</SelectItem>
              {PRESET_DAYS.map((d) => (
                <SelectItem key={d} value={String(d)}>
                  After {d} days
                </SelectItem>
              ))}
              {/* A custom value stored outside the presets still renders truthfully. */}
              {typeof shown === 'number' && !PRESET_DAYS.includes(shown) && (
                <SelectItem value={String(shown)}>After {shown} days</SelectItem>
              )}
              <SelectItem value="never">Never</SelectItem>
            </SelectContent>
          </Select>
        }
      />
      <RetentionChangeDialog
        pending={pending}
        onConfirm={handleConfirm}
        onCancel={() => setPending(null)}
      />
    </>
  );
}
