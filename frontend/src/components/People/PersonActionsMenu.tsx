'use client';

/**
 * PersonActionsMenu — the one overflow menu for a person, shared by the People directory row
 * and the person page header (it was duplicated in both). Edit and "Manage voice samples" are
 * optional: the person page has its own name/notes editors and a Voice samples tab, so it
 * passes neither.
 *
 * Order: Edit, Manage voice samples, separator, the voice opt-out toggle (a trailing check
 * marks the active "don't store" state), separator, the destructive Forget.
 */

import { DeckIcon } from '@/components/ui/deck-icon';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Check, MicOff, Pencil, Trash2, Waves } from '@/components/ui/icons';
import { MenuButton, type MenuButtonProps } from '@/components/ui/menu-button';

export interface PersonActionsMenuProps {
  /** The person's current voice opt-out state (drives the check and hides voice samples). */
  voiceprintOptOut: boolean;
  /** Edit in the form dialog. Omitted on the person page. */
  onEdit?: () => void;
  /** Open the voice-samples dialog. Omitted on the person page (it has a tab). */
  onManageVoice?: () => void;
  onToggleVoiceOptOut: () => void;
  /** The opt-out write is in flight: the item is disabled. */
  optOutSaving?: boolean;
  onForget: () => void;
  /** Trigger size: `sm` (28px) in a row, `md` (32px) in a page header. */
  size?: MenuButtonProps['size'];
}

export function PersonActionsMenu({
  voiceprintOptOut,
  onEdit,
  onManageVoice,
  onToggleVoiceOptOut,
  optOutSaving = false,
  onForget,
  size,
}: PersonActionsMenuProps) {
  const hasTopGroup = !!onEdit || (!!onManageVoice && !voiceprintOptOut);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <MenuButton label="Person options" size={size} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {onEdit && (
          <DropdownMenuItem icon={Pencil} onSelect={onEdit}>
            Edit
          </DropdownMenuItem>
        )}
        {/* specs/0039 WS3 — per-sample voice gallery management (quarantine/restore/delete).
            Hidden when this person's voice isn't stored (opt-out ⇒ no samples). */}
        {onManageVoice && !voiceprintOptOut && (
          <DropdownMenuItem icon={Waves} onSelect={onManageVoice}>
            Manage voice samples
          </DropdownMenuItem>
        )}
        {hasTopGroup && <DropdownMenuSeparator />}
        {/* Voiceprint opt-out — identity/naming still work; this only blocks voice
            modeling. A check marks the active ("don't store") state. */}
        <DropdownMenuItem
          icon={MicOff}
          disabled={optOutSaving}
          onSelect={(e) => {
            e.preventDefault();
            onToggleVoiceOptOut();
          }}
        >
          Don&apos;t store this person&apos;s voice
          {voiceprintOptOut && <DeckIcon icon={Check} size={16} className="ml-auto" />}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={onForget}>
          Forget this person
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
