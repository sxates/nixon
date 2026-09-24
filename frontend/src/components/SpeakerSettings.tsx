'use client';

/**
 * Transcription language and speaker settings (specs/0067).
 *
 * Moved out of Recordings, which had grown into a grab-bag of devices, language, biometric
 * consent and retention. These describe *what a transcript says and who said it*, so they
 * belong beside the engine that produces it rather than two tabs away. "Transcription
 * language" came with them: it sat under Recordings while "Summary language" sat under
 * Summary — the same setting in two places by accident.
 *
 * Voiceprints stay here rather than in a Privacy tab (owner decision, 0067): the consent
 * toggles are meaningless away from the diarization they modify, and duplicating them
 * across two tabs is the specs/0061 bug — two switches over one piece of state.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Switch } from './ui/switch';
import { Button } from './ui/button';
import { ClearVoiceprintsDialog } from '@/components/ClearVoiceprintsDialog';
import { LanguageSelection } from '@/components/LanguageSelection';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useConfig } from '@/contexts/ConfigContext';
import { noticeForSetting } from '@/lib/recording-settings-notices';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from './ui/settings';

export function SpeakerSettings() {
  /**
   * Transcription language is shown only for engines that honour it (specs/0067).
   *
   * On the default engine it controls nothing: `parakeet_provider.rs` takes the language,
   * logs "Parakeet doesn't support language preference … yet", and transcribes anyway.
   * The UI reflected that by filtering the menu down to Auto — and then still spent a
   * dropdown, a "Parakeet language support" note, a Current: line and an accuracy warning
   * saying so. Four blocks of text for a setting with no effect.
   *
   * Whisper and the cloud providers do use it, and the app's own advice there is to pick a
   * specific language for best accuracy — so the control stays for them rather than being
   * deleted outright, which would strand anyone transcribing in a language Whisper needs
   * told about.
   */
  const { transcriptModelConfig } = useConfig();
  const engineUsesLanguage = transcriptModelConfig.provider !== 'parakeet';
  // Same rule Recording settings uses: a change made mid-meeting applies to the NEXT
  // recording, and the toast has to say so instead of claiming it took effect now.
  const { activeRecordingMeetingId } = useSidebar();
  const isRecordingActive =
    !!activeRecordingMeetingId && activeRecordingMeetingId !== 'intro-call';
  const toastSaved = (enabled: boolean) => {
    // specs/0076: switching live labels OFF also stops them in the meeting under way, so
    // the "applies to your next recording" notice is only true when switching them on.
    if (!enabled && isRecordingActive) {
      toast.success('Live speaker labels are off', {
        description: 'Stopped for this meeting too. Speakers are still labelled after it ends.',
      });
      return;
    }
    const notice = noticeForSetting('live-diarization', isRecordingActive);
    if (notice) {
      toast.info(notice.title, { description: notice.description, duration: 8000 });
    } else {
      toast.success('Preference saved');
    }
  };

  // Voiceprint consent (specs/0016 1c, ADR-0007 as amended by specs/0078). One
  // off-by-default opt-in covers every voiceprint, your own included. It lives in
  // DiarizationSettings and is only meaningful when diarization is enabled.
  // Live (during-recording) diarization sub-toggle (specs/0011, P3-B). Default off;
  // visually subordinate to / gated by the main diarization-enabled setting above.
  // Speaker diarization (specs/0010). Opt-in / default OFF.
  const [diarizationEnabled, setDiarizationEnabled] = useState(false);
  const [liveDiarizationEnabled, setLiveDiarizationEnabled] = useState(false);
  const [storeVoiceprints, setStoreVoiceprints] = useState(false);
  const [clearVoiceprintsOpen, setClearVoiceprintsOpen] = useState(false);

  // Load speaker diarization preference (default off; specs/0010).
  useEffect(() => {
    const loadDiarizationEnabled = async () => {
      try {
        const enabled = await invoke<boolean>('api_get_diarization_enabled');
        setDiarizationEnabled(enabled);
      } catch (error) {
        console.error('Failed to load diarization preference:', error);
      }
    };
    loadDiarizationEnabled();
  }, []);

  // Load live diarization preference (default off; specs/0011, P3-B).
  useEffect(() => {
    const loadLiveDiarizationEnabled = async () => {
      try {
        const enabled = await invoke<boolean>('api_get_live_diarization_enabled');
        setLiveDiarizationEnabled(enabled);
      } catch (error) {
        console.error('Failed to load live diarization preference:', error);
      }
    };
    loadLiveDiarizationEnabled();
  }, []);

  // Load the voiceprint consent (specs/0016 1c; default false).
  useEffect(() => {
    const loadVoiceprintSettings = async () => {
      try {
        const dto = await invoke<{ storeVoiceprints: boolean }>('api_get_voiceprint_settings');
        setStoreVoiceprints(!!dto.storeVoiceprints);
      } catch (error) {
        console.error('Failed to load voiceprint settings:', error);
      }
    };
    loadVoiceprintSettings();
  }, []);

  const handleDiarizationToggle = async (enabled: boolean) => {
    const previous = diarizationEnabled;
    setDiarizationEnabled(enabled);
    try {
      await invoke('api_set_diarization_enabled', { enabled });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save diarization preference:', error);
      setDiarizationEnabled(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  const handleLiveDiarizationToggle = async (enabled: boolean) => {
    const previous = liveDiarizationEnabled;
    setLiveDiarizationEnabled(enabled);
    try {
      await invoke('api_set_live_diarization_enabled', { enabled });
      toastSaved(enabled);
    } catch (error) {
      console.error('Failed to save live diarization preference:', error);
      setLiveDiarizationEnabled(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // The one voiceprint consent (ADR-0007 §2, amended by specs/0078): your own voice and
  // other people's. Off by default; turning it off doesn't delete already-stored samples
  // (that's "clear all" / per-person opt-out) — it only blocks future enrollment.
  const handleStoreVoiceprintsToggle = async (enabled: boolean) => {
    const previous = storeVoiceprints;
    setStoreVoiceprints(enabled);
    try {
      await invoke('api_set_store_voiceprints', { enabled });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save voiceprint setting:', error);
      setStoreVoiceprints(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  return (
    <div className="space-y-8">
      {engineUsesLanguage && (
        <SettingsSection
          title="Transcription language"
          description="The single global language preference used for every transcript."
        >
          <SettingsGroup className="py-4">
            <LanguageSelection />
          </SettingsGroup>
        </SettingsSection>
      )}

      <SettingsSection
        title="Speaker labels"
        description="Who said what. Diarization runs on-device after the meeting; voiceprints never leave this Mac."
      >
        <SettingsGroup>
          {/* Speaker diarization (specs/0010) — opt-in, default off. */}
          <SettingsRow
            label="Speaker diarization"
            description="Label who spoke in the transcript. Runs on-device after the meeting and downloads a small model (~108 MB) the first time."
            control={
              <Switch
                checked={diarizationEnabled}
                onCheckedChange={handleDiarizationToggle}
                aria-label="Speaker diarization"
              />
            }
          />

          {/* Live speaker labels (specs/0011, P3-B) and the expected-count override are
              sub-settings of the enable above: they only exist once it is on. */}
          {diarizationEnabled && (
            <SettingsRow
              label="Label speakers live while recording"
              description="Shows provisional numbered labels while you record. Names are matched when the recording ends. Uses significant CPU."
              control={
                <Switch
                  checked={liveDiarizationEnabled}
                  onCheckedChange={handleLiveDiarizationToggle}
                  aria-label="Label speakers live while recording"
                />
              }
            />
          )}

          {/* Voice identification / voiceprints (specs/0016 1c, ADR-0007, specs/0078).
              Only meaningful when diarization is on, since cross-meeting voice memory is
              built from diarized speakers. One consent, OFF by default. */}
          {diarizationEnabled && (
            <SettingsRow
              label="Store voiceprints"
              description={
                <>
                  Off by default. Covers your own voice and other people&apos;s. Voiceprints
                  are biometric data, so storing them is opt-in. When on, Nixon remembers
                  voices to label &quot;You&quot; and suggest names automatically in future
                  meetings. When off, names still suggest within a single meeting, but no
                  cross-meeting voice memory is kept. Voiceprints stay on this Mac either way.
                </>
              }
              control={
                <Switch
                  checked={storeVoiceprints}
                  onCheckedChange={handleStoreVoiceprintsToggle}
                  aria-label="Store voiceprints"
                />
              }
            />
          )}

          {/* Destructive: wipe the entire gallery. Last row of the section. */}
          {diarizationEnabled && (
            <SettingsRow
              label="Clear all voiceprints"
              description="Delete every stored voice sample. People and their names are kept; Nixon just re-learns voices from scratch."
              control={
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => setClearVoiceprintsOpen(true)}
                >
                  Clear all
                </Button>
              }
            />
          )}
        </SettingsGroup>
      </SettingsSection>

      <ClearVoiceprintsDialog
        open={clearVoiceprintsOpen}
        onOpenChange={setClearVoiceprintsOpen}
      />
    </div>
  );
}
