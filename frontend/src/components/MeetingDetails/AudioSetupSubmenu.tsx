"use client";

/**
 * "Who was on the mic?" (specs/0078 W3) — a submenu of the transcript's "…" menu.
 *
 * A call has one voice on the mic (you) and everyone else on system audio; a room
 * recording has everyone on the one shared mic. Nixon detects which it was, and this is
 * where the user overrides a wrong guess. Choosing an option re-runs speaker
 * identification, so the parent hands the choice to the diarization controller.
 */

import {
  DropdownMenuPortal,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu';
import type { AudioSetupOverride, MeetingAudioSetup } from '@/hooks/useAudioSetup';

const DETECTED_CAPTION: Record<NonNullable<MeetingAudioSetup['resolved']>, string> = {
  room: 'detected: in a room',
  call: 'detected: on a call',
  hybrid: 'detected: room and call',
};

interface AudioSetupSubmenuProps {
  setup: MeetingAudioSetup | null;
  /** A pass is running: the choice would only apply to the next one. */
  disabled: boolean;
  onChoose: (setup: AudioSetupOverride) => void;
}

export function AudioSetupSubmenu({ setup, disabled, onChoose }: AudioSetupSubmenuProps) {
  const value = setup?.override ?? 'auto';
  // The caption reports what DETECTION decided, so it only means something when the
  // last pass wasn't forced by an override.
  const caption =
    value === 'auto' && setup?.resolved ? DETECTED_CAPTION[setup.resolved] : null;

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger
        disabled={disabled}
        className="data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
        title={disabled ? 'Available once speaker identification finishes' : undefined}
      >
        Who was on the mic?
      </DropdownMenuSubTrigger>
      <DropdownMenuPortal>
        <DropdownMenuSubContent className="w-72">
          <DropdownMenuRadioGroup
            value={value}
            onValueChange={(v) => onChoose(v as AudioSetupOverride)}
          >
            <DropdownMenuRadioItem value="auto" className="flex-col items-start">
              <span>Detect automatically</span>
              {caption && (
                <span className="text-xs text-muted-foreground">{caption}</span>
              )}
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="room">
              Everyone in the room, one shared mic
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="call">
              Just me, others on the call
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuPortal>
    </DropdownMenuSub>
  );
}
