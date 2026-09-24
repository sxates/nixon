"use client";

/**
 * "This is me" / "This isn't me" (specs/0078 W3).
 *
 * On a call, "You" is simply whoever was on the mic. In a room recording everyone shares
 * the mic, so the owner is one cluster among several and the user may have to say which.
 * These actions sit in the speaker's rename/assign popover (the legend chip and the
 * transcript line's name), where someone naming a speaker already looks.
 *
 * Rules:
 *  - "This is me" on any speaker other than "You" and "Unknown speaker", when the last
 *    pass was a room (or hybrid) pass, OR when the meeting has no "You" at all (so it is
 *    reachable on meetings that haven't been re-identified since room detection shipped).
 *  - "This isn't me" on "You", only for room/hybrid passes: in a call "You" is the mic.
 */

import { UserCheck, UserX } from 'lucide-react';
import { isRoomSetup, type AudioSetupResolved } from '@/hooks/useAudioSetup';

const LOCAL_KEY = 'local';
const UNKNOWN_KEY = 'unknown';

/** What a speaker popover needs to offer the owner actions. */
export interface OwnerActionContext {
  /** The setup the last diarization pass used; null = not recorded yet. */
  resolved: AudioSetupResolved | null;
  /** Whether the meeting already has a "You" speaker. */
  hasLocalSpeaker: boolean;
  markAsMe: (speakerKeys: string | string[]) => Promise<void>;
  unmarkMe: () => Promise<void>;
}

/** Build the context from the shared speakers controller's fields. */
export function ownerActionContext(c: {
  speakers: { speakerKey: string; isLocal: boolean }[];
  audioSetup?: { resolved: AudioSetupResolved | null } | null;
  markAsMe: OwnerActionContext['markAsMe'];
  unmarkMe: OwnerActionContext['unmarkMe'];
}): OwnerActionContext {
  return {
    resolved: c.audioSetup?.resolved ?? null,
    hasLocalSpeaker: c.speakers.some((s) => s.isLocal || s.speakerKey === LOCAL_KEY),
    markAsMe: c.markAsMe,
    unmarkMe: c.unmarkMe,
  };
}

/** Which owner action (if any) a speaker gets. Pure, for the tests. */
export function ownerActionFor(
  speakerKey: string,
  isLocal: boolean,
  ctx: Pick<OwnerActionContext, 'resolved' | 'hasLocalSpeaker'>,
): 'mark' | 'unmark' | null {
  const room = isRoomSetup(ctx.resolved);
  if (isLocal || speakerKey === LOCAL_KEY) return room ? 'unmark' : null;
  if (speakerKey === UNKNOWN_KEY) return null;
  return room || !ctx.hasLocalSpeaker ? 'mark' : null;
}

interface SpeakerOwnerActionProps {
  speakerKey: string;
  isLocal?: boolean;
  /** Every key the chip stands for (a consolidated group); defaults to `[speakerKey]`. */
  memberKeys?: string[];
  owner: OwnerActionContext | undefined;
  /** Called before the action runs, e.g. to close the popover. */
  onBeforeAction?: () => void;
}

export function SpeakerOwnerAction({
  speakerKey,
  isLocal = false,
  memberKeys,
  owner,
  onBeforeAction,
}: SpeakerOwnerActionProps) {
  if (!owner) return null;
  const action = ownerActionFor(speakerKey, isLocal, owner);
  if (!action) return null;

  const run = () => {
    onBeforeAction?.();
    if (action === 'mark') void owner.markAsMe(memberKeys ?? [speakerKey]);
    else void owner.unmarkMe();
  };

  const Icon = action === 'mark' ? UserCheck : UserX;
  return (
    <button
      type="button"
      onClick={run}
      className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs text-foreground hover:bg-muted"
      title={
        action === 'mark'
          ? 'Make this speaker "You" in this meeting'
          : 'This speaker becomes a numbered speaker again'
      }
    >
      <Icon size={13} className="flex-shrink-0 text-muted-foreground" />
      {action === 'mark' ? 'This is me' : "This isn't me"}
    </button>
  );
}

/** One line under the legend for a room meeting with no "You" yet. */
export function RoomOwnerHint({ owner }: { owner: OwnerActionContext }) {
  if (!isRoomSetup(owner.resolved) || owner.hasLocalSpeaker) return null;
  return (
    <p className="text-xs text-muted-foreground">
      Recorded in a room. Mark which speaker is you: click the name, then &ldquo;This is
      me&rdquo;.
    </p>
  );
}
