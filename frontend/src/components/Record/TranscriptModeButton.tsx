'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { BatteryLow } from '@/components/ui/icons';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { useProcessingMode } from '@/hooks/useProcessingMode';
import { modeChipDisplay } from '@/lib/processing-mode';
import { STRIP_KEY_CLASS } from './RecordMeetingControls';

/**
 * Compact per-meeting live/defer mode chip (low-power-mode spec §5). Driven by
 * `useProcessingMode()`'s live `processing-mode-changed` event; clicking flips the
 * mode for the current recording via `api_set_meeting_processing_mode` +
 * `api_apply_live_transcription_now`. Renders nothing until the first event of this
 * session has arrived, so it never flashes a guessed state.
 *
 * Lived in the recording header until v0.18; the owner moved it into the transcript's own
 * status strip (TranscriptPanel), since pausing the transcript only matters next to it.
 */
export function TranscriptModeButton({ meetingId }: { meetingId: string }) {
  const { liveTranscription, onBattery } = useProcessingMode();
  // Only the live-going transition waits on the STT model spinning up; deferring is
  // effectively instant. Track it separately so the tooltip copy stays accurate.
  const [enabling, setEnabling] = useState(false);

  if (liveTranscription === null) return null;
  const { label, showBatteryGlyph } = modeChipDisplay(liveTranscription, onBattery);

  const handleClick = async () => {
    if (enabling) return;
    const goingLive = !liveTranscription;
    if (goingLive) setEnabling(true);
    try {
      await invoke('api_set_meeting_processing_mode', {
        meetingId,
        mode: goingLive ? 'live' : 'defer',
      });
      await invoke('api_apply_live_transcription_now', { enable: goingLive });
    } catch (error) {
      console.error('Failed to change this meeting\'s transcription mode:', error);
      toast.error('Failed to change transcription mode');
    } finally {
      if (goingLive) setEnabling(false);
    }
  };

  const chip = (
    <Button
      type="button"
      variant="outline"
      size="xs"
      onClick={handleClick}
      disabled={enabling}
      // The action, matching the visible label — a screen reader must hear the same thing an
      // eye reads (specs/0071 W1). It used to announce the mode and then the switch, which is
      // the same trap in longer form.
      aria-label={label}
      className={STRIP_KEY_CLASS}
    >
      {/* The only carrier of WHY the transcript is paused (on battery): functional, not
          decorative, so it stays. */}
      {showBatteryGlyph && <DeckIcon icon={BatteryLow} size={14} />}
      {enabling ? 'Switching…' : label}
    </Button>
  );

  if (!enabling) return chip;

  return (
    <TooltipProvider>
      <Tooltip open>
        <TooltipTrigger asChild>{chip}</TooltipTrigger>
        <TooltipContent>waiting for model…</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
