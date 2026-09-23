'use client';

/**
 * "Save location" settings row (specs/0061 W6, specs/0073 W3).
 *
 * Nixon keeps every recording in one folder, so "Change…" moves the existing recordings
 * too: it plans the move, asks with MoveRecordingsDialog (Move recordings / Cancel), then
 * hands the whole job to Rust (`api_change_recordings_folder`). With nothing to move there
 * is no question — the folder just changes. Progress, Stop and the "still in another
 * folder" line come from useRecordingsMove, which re-attaches to a running move on mount.
 */

import { useState, type Dispatch, type SetStateAction } from 'react';
import { FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { SettingsRow } from '@/components/ui/settings';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { MoveRecordingsDialog } from '@/components/MoveRecordingsDialog';
import { useRecordingsMove } from '@/hooks/useRecordingsMove';
import { tildePath } from '@/lib/format-path';
import {
  errorText,
  isICloudDrivePath,
  isMovePlan,
  progressLine,
  progressPercent,
  type MovePlan,
} from '@/lib/recordings-move';
import type { RecordingPreferences } from './RecordingSettings';

export const STOP_RECORDING_FIRST = 'Stop the recording to change where recordings are saved';

interface SaveLocationRowProps {
  preferences: RecordingPreferences;
  setPreferences: Dispatch<SetStateAction<RecordingPreferences>>;
  onSave?: (preferences: RecordingPreferences) => void;
  /** A recording is running: the folder can't change under it. */
  disabled?: boolean;
}

export function SaveLocationRow({
  preferences,
  setPreferences,
  onSave,
  disabled = false,
}: SaveLocationRowProps) {
  const move = useRecordingsMove();
  const [pending, setPending] = useState<{ target: string; plan: MovePlan } | null>(null);

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
    }
  };

  // Rust persists the folder (and starts any move) before this resolves; mirror it locally.
  const changeTo = async (target: string) => {
    await invoke('api_change_recordings_folder', { target });
    const next = { ...preferences, save_folder: target, save_folder_user_chosen: true };
    setPreferences(next);
    onSave?.(next);
    void move.refreshGather();
  };

  const handleChangeFolder = async () => {
    let picked: string | null;
    try {
      picked = await invoke<string | null>('select_recording_folder');
    } catch (error) {
      console.error('Failed to open folder picker:', error);
      toast.error('Failed to open folder picker');
      return;
    }
    if (!picked) return; // user cancelled

    let plan: unknown;
    try {
      plan = await invoke('api_plan_recordings_move', { target: picked });
    } catch (error) {
      toast.error(errorText(error));
      return;
    }
    if (!isMovePlan(plan)) {
      toast.error("Nixon couldn't check the recordings for that folder.");
      return;
    }
    if (plan.meetings > 0) {
      setPending({ target: picked, plan });
      return;
    }
    // Nothing to move: no question, the folder just changes.
    try {
      await changeTo(picked);
      toast.success('Recordings folder changed', {
        description: isICloudDrivePath(picked)
          ? 'iCloud Drive can remove the copies on this Mac to save space. Nixon needs the files on this Mac.'
          : `New recordings will be saved to ${tildePath(picked)}.`,
      });
    } catch (error) {
      toast.error(errorText(error));
    }
  };

  const changeButton = (
    <Button
      variant="outline"
      size="sm"
      onClick={handleChangeFolder}
      disabled={disabled || !!move.status}
    >
      Change…
    </Button>
  );

  return (
    <>
      <SettingsRow
        label="Save location"
        description={
          // Rendered through `tildePath` so the row never puts the user's account name on
          // screen (it reached a committed screenshot once). The full path is one hover away,
          // and every path we ACT on is the unabbreviated `save_folder`.
          <span className="break-all" title={preferences.save_folder || undefined}>
            {preferences.save_folder ? tildePath(preferences.save_folder) : 'Default folder'}
          </span>
        }
        control={
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={handleOpenFolder}>
              <FolderOpen className="h-4 w-4" />
              Open folder
            </Button>
            {disabled ? (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* A disabled button fires no pointer events; the span carries the hover. */}
                    <span tabIndex={0} aria-label={STOP_RECORDING_FIRST}>
                      {changeButton}
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{STOP_RECORDING_FIRST}</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            ) : (
              changeButton
            )}
          </div>
        }
      >
        {move.status ? (
          <div className="mt-3 space-y-2" data-testid="recordings-move-progress">
            <Progress value={progressPercent(move.status)} className="h-1.5" />
            <div className="flex items-center justify-between gap-4">
              <p className="min-w-0 truncate text-xs text-muted-foreground" aria-live="polite">
                {progressLine(move.status)}
              </p>
              <Button variant="outline" size="sm" onClick={move.stop} disabled={move.stopping}>
                {move.stopping ? 'Stopping…' : 'Stop'}
              </Button>
            </div>
          </div>
        ) : move.leftBehind > 0 ? (
          <div className="mt-2 text-xs text-muted-foreground" data-testid="recordings-left-behind">
            <p>
              {move.leftBehind === 1
                ? '1 recording is still in another folder — '
                : `${move.leftBehind} recordings are still in another folder — `}
              <button
                type="button"
                onClick={move.gatherHere}
                disabled={disabled}
                className="font-semibold text-brand underline-offset-2 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              >
                Move them here
              </button>
            </p>
            {move.leftBehindReason && <p className="mt-1">{move.leftBehindReason}</p>}
          </div>
        ) : null}
      </SettingsRow>

      <MoveRecordingsDialog
        open={!!pending}
        onOpenChange={(open) => !open && setPending(null)}
        plan={pending?.plan ?? null}
        mode="change"
        onConfirm={() => (pending ? changeTo(pending.target) : Promise.resolve())}
      />
    </>
  );
}
