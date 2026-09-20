import React, { useState, useEffect } from 'react';
import { Switch } from '@/components/ui/switch';
import ZoomMuteGateToggle from '@/components/ZoomMuteGateToggle';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import { LanguageSelection } from '@/components/LanguageSelection';
import { SaveLocationRow } from '@/components/SaveLocationRow';
import { Button } from '@/components/ui/button';
import { ClearVoiceprintsDialog } from '@/components/ClearVoiceprintsDialog';
import {
  applyRetentionChoice,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
} from '@/lib/audio-retention';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  noticeForSetting,
  type StartTimeOnlySetting,
} from '@/lib/recording-settings-notices';
import {
  SettingsGroup,
  SettingsNote,
  SettingsRow,
  SettingsSection,
} from '@/components/ui/settings';

export interface RecordingPreferences {
  save_folder: string;
  auto_save: boolean;
  preferred_mic_device: string | null;
  preferred_system_device: string | null;
  /** Auto-delete recording audio after N days (specs/0029 WS7.1); null = keep forever. */
  retention_days: number | null;
  /** Real-time transcription while recording (specs/0029 WS7.2). false = record-only
   *  mode: audio is saved but STT is deferred (transcribe later / at summarize). */
  live_transcription_enabled: boolean;
  /** Low Power Mode (low-power-mode spec): on battery, record audio only and defer
   *  transcription + summaries until back on power. Overridable per meeting while
   *  recording. Backend serde default is true. */
  low_power_on_battery: boolean;
}

interface RecordingSettingsProps {
  onSave?: (preferences: RecordingPreferences) => void;
}

export function RecordingSettings({ onSave }: RecordingSettingsProps) {
  // spec 0051 WS3: `activeRecordingMeetingId` is the recording id that survives
  // navigating away from /record — which is exactly where the user is when they change
  // a setting mid-meeting. The 'intro-call' placeholder is not a real recording.
  const { activeRecordingMeetingId } = useSidebar();
  const isRecordingActive =
    !!activeRecordingMeetingId && activeRecordingMeetingId !== 'intro-call';

  /**
   * Success toast that tells the truth about when the change takes effect.
   *
   * While a recording is active, the setting's "applies to your next recording" notice
   * wins. Otherwise the change genuinely does apply now, so callers with something more
   * useful to say than a bare "Preference saved" pass it as `fallbackDescription` (e.g.
   * live-transcription's and low-power's state-dependent explanation) — losing that
   * description in the common no-active-recording case would be a regression, not a fix.
   */
  const toastSaved = (setting: StartTimeOnlySetting, fallbackDescription?: string) => {
    const notice = noticeForSetting(setting, isRecordingActive);
    if (notice) {
      toast.info(notice.title, { description: notice.description, duration: 8000 });
    } else if (fallbackDescription) {
      toast.success('Preference saved', { description: fallbackDescription });
    } else {
      toast.success('Preference saved');
    }
  };

  const [preferences, setPreferences] = useState<RecordingPreferences>({
    save_folder: '',
    auto_save: true,
    preferred_mic_device: null,
    preferred_system_device: null,
    retention_days: null,
    live_transcription_enabled: true,
    low_power_on_battery: true
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [showRecordingNotification, setShowRecordingNotification] = useState(true);
  const [zoomAutoDetect, setZoomAutoDetect] = useState(true);
  // Speaker diarization (specs/0010). Opt-in / default OFF.
  const [diarizationEnabled, setDiarizationEnabled] = useState(false);
  // Live (during-recording) diarization sub-toggle (specs/0011, P3-B). Default off;
  // visually subordinate to / gated by the main diarization-enabled setting above.
  const [liveDiarizationEnabled, setLiveDiarizationEnabled] = useState(false);
  // Voiceprint consent controls (specs/0016 1c, ADR-0007). `storeOthers` is the
  // off-by-default global opt-in to persist *other people's* voiceprints; `selfEnroll`
  // is the device-owner ("You") self-enroll, on by default. Both live in
  // DiarizationSettings and are only meaningful when diarization is enabled.
  const [storeOthersVoiceprints, setStoreOthersVoiceprints] = useState(false);
  const [selfEnrollVoiceprint, setSelfEnrollVoiceprint] = useState(true);
  const [clearVoiceprintsOpen, setClearVoiceprintsOpen] = useState(false);
  // Owner emails (specs/0018) moved to Settings → General (OwnerEmailSettings),
  // and notification controls now live only on the General tab.

  // Load recording preferences on component mount
  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
        setPreferences(prefs);
      } catch (error) {
        console.error('Failed to load recording preferences:', error);
        // If loading fails, get default folder path
        try {
          const defaultPath = await invoke<string>('get_default_recordings_folder_path');
          setPreferences(prev => ({ ...prev, save_folder: defaultPath }));
        } catch (defaultError) {
          console.error('Failed to get default folder path:', defaultError);
        }
      } finally {
        setLoading(false);
      }
    };

    loadPreferences();
  }, []);

  // Load recording notification preference
  useEffect(() => {
    const loadNotificationPref = async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const show = await store.get<boolean>('show_recording_notification') ?? true;
        setShowRecordingNotification(show);
      } catch (error) {
        console.error('Failed to load notification preference:', error);
      }
    };
    loadNotificationPref();
  }, []);

  // Load Zoom auto-detect preference (default on)
  useEffect(() => {
    const loadZoomAutoDetect = async () => {
      try {
        const enabled = await invoke<boolean>('api_get_zoom_auto_detect');
        setZoomAutoDetect(enabled);
      } catch (error) {
        console.error('Failed to load Zoom auto-detect preference:', error);
      }
    };
    loadZoomAutoDetect();
  }, []);

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

  // Load the two voiceprint-consent toggles (specs/0016 1c). Both come back in a
  // single DTO: storeOthers (default false) + selfEnroll (default true).
  useEffect(() => {
    const loadVoiceprintSettings = async () => {
      try {
        const dto = await invoke<{
          storeOthersVoiceprints: boolean;
          selfEnrollVoiceprint: boolean;
        }>('api_get_voiceprint_settings');
        setStoreOthersVoiceprints(!!dto.storeOthersVoiceprints);
        setSelfEnrollVoiceprint(!!dto.selfEnrollVoiceprint);
      } catch (error) {
        console.error('Failed to load voiceprint settings:', error);
      }
    };
    loadVoiceprintSettings();
  }, []);

  const handleZoomAutoDetectToggle = async (enabled: boolean) => {
    const previous = zoomAutoDetect;
    setZoomAutoDetect(enabled);
    try {
      await invoke('api_set_zoom_auto_detect', { enabled });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save Zoom auto-detect preference:', error);
      setZoomAutoDetect(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

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
      toastSaved('live-diarization');
    } catch (error) {
      console.error('Failed to save live diarization preference:', error);
      setLiveDiarizationEnabled(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // Global opt-in to store *other people's* voiceprints (ADR-0007 §2). Off by default;
  // turning it off doesn't delete already-stored samples (that's "clear all" / per-person
  // opt-out) — it only blocks future enrollment of non-owner people.
  const handleStoreOthersToggle = async (enabled: boolean) => {
    const previous = storeOthersVoiceprints;
    setStoreOthersVoiceprints(enabled);
    try {
      await invoke('api_set_store_others_voiceprints', { enabled });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save voiceprint setting:', error);
      setStoreOthersVoiceprints(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // Device-owner ("You") self-enroll (ADR-0007 §3). On by default.
  const handleSelfEnrollToggle = async (enabled: boolean) => {
    const previous = selfEnrollVoiceprint;
    setSelfEnrollVoiceprint(enabled);
    try {
      await invoke('api_set_self_enroll_voiceprint', { enabled });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save self-enroll setting:', error);
      setSelfEnrollVoiceprint(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // Audio retention (specs/0029 WS7.1) — ONE control ("Delete audio recordings":
  // Immediately / after N days / Never) mapped onto the UNCHANGED backend pair
  // { auto_save, retention_days } via lib/audio-retention.ts. Writes through the
  // same recording-preferences store the folder/device settings use; the backend
  // sweep re-reads it on every pass, so no relaunch is needed. Optimistic, revert
  // on failure. Kept separate from savePreferences() so the toast copy is
  // truthful about WHAT gets deleted.
  const handleRetentionChange = async (value: string) => {
    const choice = retentionChoiceFromSelectValue(value);
    const previous = preferences;
    const newPreferences = applyRetentionChoice(preferences, choice);
    setPreferences(newPreferences);
    try {
      await invoke('set_recording_preferences', { preferences: newPreferences });
      onSave?.(newPreferences);
      toast.success('Preference saved', {
        description:
          choice === 'immediately'
            ? 'Audio is discarded when the recording stops. Transcripts, notes, and summaries are still saved.'
            : choice === 'never'
              ? 'Recording audio is kept forever.'
              : `Audio files older than ${choice} days will be deleted. Transcripts, notes, and summaries are always kept.`,
      });
    } catch (error) {
      console.error('Failed to save audio retention preference:', error);
      setPreferences(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // Live transcription during recording (specs/0029 WS7.2). Optimistic, revert on
  // failure — same recording-preferences store as folder/device settings; the backend
  // reads it at every recording start, so no relaunch is needed.
  const handleLiveTranscriptionToggle = async (enabled: boolean) => {
    const previous = preferences;
    const newPreferences = { ...preferences, live_transcription_enabled: enabled };
    setPreferences(newPreferences);
    try {
      await invoke('set_recording_preferences', { preferences: newPreferences });
      onSave?.(newPreferences);
      toastSaved(
        'live-transcription',
        enabled
          ? 'Meetings will be transcribed live while you record.'
          : 'Recording only — meetings are transcribed later (automatically before a summary, or with “Transcribe now”).',
      );
    } catch (error) {
      console.error('Failed to save live transcription preference:', error);
      setPreferences(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  // Low Power Mode on battery (low-power-mode spec). Optimistic, revert on failure —
  // same recording-preferences store as the other toggles here. Applies to future
  // recording sessions; an in-progress recording is controlled separately via the
  // per-meeting chip in the recording header.
  const handleLowPowerToggle = async (enabled: boolean) => {
    const previous = preferences;
    const newPreferences = { ...preferences, low_power_on_battery: enabled };
    setPreferences(newPreferences);
    try {
      await invoke('set_recording_preferences', { preferences: newPreferences });
      onSave?.(newPreferences);
      toastSaved(
        'low-power-on-battery',
        enabled
          ? "On battery power, meetings are recorded only — transcription and summaries wait until you're plugged in."
          : 'Meetings are transcribed live even on battery.',
      );
    } catch (error) {
      console.error('Failed to save low power mode preference:', error);
      setPreferences(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  const handleDeviceChange = async (devices: SelectedDevices) => {
    const newPreferences = {
      ...preferences,
      preferred_mic_device: devices.micDevice,
      preferred_system_device: devices.systemDevice
    };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleNotificationToggle = async (enabled: boolean) => {
    try {
      setShowRecordingNotification(enabled);
      const { Store } = await import('@tauri-apps/plugin-store');
      const store = await Store.load('preferences.json');
      await store.set('show_recording_notification', enabled);
      await store.save();
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save notification preference:', error);
      toast.error('Failed to save preference');
    }
  };

  const savePreferences = async (prefs: RecordingPreferences) => {
    setSaving(true);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      onSave?.(prefs);

      // Show success toast with device details
      const micDevice = prefs.preferred_mic_device || 'Default';
      const systemDevice = prefs.preferred_system_device || 'Default';
      toast.success("Device preferences saved", {
        description: `Microphone: ${micDevice}, System Audio: ${systemDevice}`
      });
    } catch (error) {
      console.error('Failed to save recording preferences:', error);
      toast.error("Failed to save device preferences", {
        description: error instanceof Error ? error.message : String(error)
      });
    } finally {
      setSaving(false);
    }
  };

  // What the single "Delete audio recordings" select shows for the stored
  // { auto_save, retention_days } pair. auto_save=false always reads as
  // "Immediately", regardless of any leftover retention value.
  const retentionChoice = retentionChoiceFromPreferences(preferences);

  // Sections run in the order a new user needs them: what is captured → how a
  // recording behaves → what language → who is labelled → where the audio lives
  // (and the destructive cleanup last).
  return (
    <div className="space-y-8">
      <SettingsSection
        title="Audio devices"
        description="Which microphone and system-audio source new recordings start with."
      >
        <SettingsGroup>
          <SettingsRow
            label="Default devices"
            description="Used automatically when you start a new recording. System audio is what captures the other participants."
            align="start"
          >
            <DeviceSelection
              selectedDevices={{
                micDevice: preferences.preferred_mic_device,
                systemDevice: preferences.preferred_system_device,
              }}
              onDeviceChange={handleDeviceChange}
              disabled={saving || loading}
            />
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>

      <SettingsSection
        title="Recording"
        description="What Nixon does while a meeting is being recorded, and right after it ends."
      >
        <SettingsGroup>
          {/* Live transcription toggle (specs/0029 WS7.2). Off = record-only mode:
              audio is still recorded (and the visualizers still work), but the STT
              stage is skipped to save CPU/battery; the meeting is transcribed later. */}
          <SettingsRow
            label="Transcribe in real time during recording"
            description={
              <>
                Show a live transcript while you record. Turning this off saves CPU and
                battery — audio is still recorded, and the meeting is transcribed later
                (automatically before a summary, or with &quot;Transcribe now&quot; on the
                meeting page).
              </>
            }
            control={
              <Switch
                checked={preferences.live_transcription_enabled}
                onCheckedChange={handleLiveTranscriptionToggle}
                disabled={saving || loading}
                aria-label="Transcribe in real time during recording"
              />
            }
          />

          {/* Low Power Mode on battery (low-power-mode spec). On by default (backend
              serde default true). Independent of the live-transcription toggle above —
              this one only kicks in on battery, and can be overridden per meeting while
              recording via the mode chip in the recording header. */}
          <SettingsRow
            label="Low Power Mode on battery"
            description={
              <>
                When on battery, record audio but defer transcription and summaries until
                you&apos;re back on power. You can override per meeting while recording.
              </>
            }
            control={
              <Switch
                checked={preferences.low_power_on_battery}
                onCheckedChange={handleLowPowerToggle}
                disabled={saving || loading}
                aria-label="Low Power Mode on battery"
              />
            }
          />

          <SettingsRow
            label="Recording start notification"
            description="Show a reminder to tell participants when recording starts."
            control={
              <Switch
                checked={showRecordingNotification}
                onCheckedChange={handleNotificationToggle}
                aria-label="Recording start notification"
              />
            }
          />

          <SettingsRow
            label="Auto-detect Zoom meetings"
            description="When a Zoom meeting starts, offer to record it."
            control={
              <Switch
                checked={zoomAutoDetect}
                onCheckedChange={handleZoomAutoDetectToggle}
                aria-label="Auto-detect Zoom meetings"
              />
            }
          />
        </SettingsGroup>

        {/* specs/0061 W6 — this used to be a SECOND "Summarize automatically" switch here,
            duplicating the one under Summary (same ConfigContext state, two surfaces to
            keep in sync). One control, one place; this just points to it. */}
        <SettingsNote tone="muted">
          Automatic summaries are configured under Summary.
        </SettingsNote>

        {/* Zoom mute gate (specs/0049) — opt-in, needs Accessibility permission.
            Renders its own ruled-row card in the same vocabulary. */}
        <ZoomMuteGateToggle />
      </SettingsSection>

      <SettingsSection
        title="Transcription language"
        description="The single global language preference used for every transcript."
      >
        <SettingsGroup className="py-4">
          <LanguageSelection />
        </SettingsGroup>
      </SettingsSection>

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
              description="Shows provisional numbered labels while you record. Names are matched when the recording ends."
              control={
                <Switch
                  checked={liveDiarizationEnabled}
                  onCheckedChange={handleLiveDiarizationToggle}
                  aria-label="Label speakers live while recording"
                />
              }
            />
          )}

          {/* Voice identification / voiceprints (specs/0016 1c, ADR-0007). Only meaningful
              when diarization is on, since cross-meeting voice memory is built from
              diarized speakers. The global "store others" gate is OFF by default. */}
          {diarizationEnabled && (
            <SettingsRow
              label="Store voiceprints for other people"
              description={
                <>
                  Off by default. Storing other people&apos;s voiceprints is opt-in because it
                  is biometric data. When on, Nixon remembers other people&apos;s voices to
                  suggest names automatically in future meetings. When off, names still suggest
                  within a single meeting, but no cross-meeting voice memory is kept for others.
                  Voiceprints stay on this Mac either way.
                </>
              }
              control={
                <Switch
                  checked={storeOthersVoiceprints}
                  onCheckedChange={handleStoreOthersToggle}
                  aria-label="Store voiceprints for other people"
                />
              }
            />
          )}

          {diarizationEnabled && (
            <SettingsRow
              label="Recognize my own voice across meetings"
              description={
                <>
                  Learns your voice (the microphone channel) so &quot;You&quot; is labeled
                  reliably in every meeting. Stored only on this Mac.
                </>
              }
              control={
                <Switch
                  checked={selfEnrollVoiceprint}
                  onCheckedChange={handleSelfEnrollToggle}
                  aria-label="Recognize my own voice across meetings"
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

      <SettingsSection
        title="Audio storage"
        description="Where the audio files live and how long they are kept. Transcripts, notes, and summaries are always kept."
      >
        <SettingsGroup>
          {/* Delete audio recordings — ONE control combining the old "Save Audio
              Recordings" toggle and the retention window. The backend contract is
              unchanged: this maps onto { auto_save, retention_days } via
              lib/audio-retention.ts ("Immediately" = auto_save off; a day count or
              Never = auto_save on + that sweep window). */}
          <SettingsRow
            label="Delete audio recordings"
            htmlFor="audio-retention"
            description={
              <>
                &quot;Immediately&quot; discards audio as soon as a recording stops. Only the
                audio is affected — meetings that haven&apos;t been transcribed yet are never
                deleted.
              </>
            }
            control={
              <select
                id="audio-retention"
                value={retentionChoiceToSelectValue(retentionChoice)}
                onChange={(e) => void handleRetentionChange(e.target.value)}
                disabled={saving || loading}
                className="rounded-md border border-input bg-background px-2 py-1 text-sm"
              >
                <option value="immediately">Immediately</option>
                <option value="7">After 7 days</option>
                <option value="30">After 30 days</option>
                <option value="90">After 90 days</option>
                {/* A custom value stored outside the presets still renders truthfully. */}
                {typeof retentionChoice === 'number' &&
                  ![7, 30, 90].includes(retentionChoice) && (
                    <option value={String(retentionChoice)}>
                      After {retentionChoice} days
                    </option>
                  )}
                <option value="never">Never</option>
              </select>
            }
          />

          {preferences.auto_save && (
            <SaveLocationRow
              preferences={preferences}
              setPreferences={setPreferences}
              onSave={onSave}
            />
          )}
        </SettingsGroup>

        {/* Info when audio is discarded immediately (auto_save off) */}
        {!preferences.auto_save && (
          <SettingsNote tone="info">
            Audio is deleted as soon as a recording stops — transcripts, notes, and
            summaries are still saved. Pick a time window (or &quot;Never&quot;) above to
            keep the audio files.
          </SettingsNote>
        )}
      </SettingsSection>

      <ClearVoiceprintsDialog
        open={clearVoiceprintsOpen}
        onOpenChange={setClearVoiceprintsOpen}
      />
    </div>
  );
}
