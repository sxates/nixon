import React, { useState, useEffect } from 'react';
import { Switch } from '@/components/ui/switch';
import ZoomMuteGateToggle from '@/components/ZoomMuteGateToggle';
import { FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import { LanguageSelection } from '@/components/LanguageSelection';
import { Button } from '@/components/ui/button';
import { ClearVoiceprintsDialog } from '@/components/ClearVoiceprintsDialog';
import {
  applyRetentionChoice,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
} from '@/lib/audio-retention';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  noticeForSetting,
  type StartTimeOnlySetting,
} from '@/lib/recording-settings-notices';

export interface RecordingPreferences {
  save_folder: string;
  auto_save: boolean;
  file_format: string;
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
  // Auto-summary on meeting end (specs/0029 WS7.3). Shared ConfigContext state — the
  // SAME `isAutoSummary` the toggle in Summary settings uses, so the two surfaces can
  // never disagree. Surfaced here too because this is where recording-lifecycle
  // options live and users look for it after a call ends.
  const { isAutoSummary, toggleIsAutoSummary } = useConfig();
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
    file_format: 'mp4',
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
  // Expected speaker count override (specs/0011 accuracy gate). Empty string = Auto
  // (let clustering decide); a number forces exactly that many speakers — the most
  // reliable fix when Auto over-/under-clusters.
  const [expectedSpeakerCount, setExpectedSpeakerCount] = useState<string>('');
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

  // Load the expected-speaker-count override (null = Auto; specs/0011).
  useEffect(() => {
    const loadExpectedSpeakerCount = async () => {
      try {
        const count = await invoke<number | null>('api_get_expected_speaker_count');
        setExpectedSpeakerCount(count != null ? String(count) : '');
      } catch (error) {
        console.error('Failed to load expected speaker count:', error);
      }
    };
    loadExpectedSpeakerCount();
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

  // Persist the expected-speaker-count override on blur (specs/0011). Empty or a
  // non-positive value clears the override (Auto); a positive integer forces that
  // many speakers. Backend normalizes 0 → Auto.
  const handleExpectedSpeakerCountCommit = async () => {
    const trimmed = expectedSpeakerCount.trim();
    const parsed = trimmed === '' ? null : Number.parseInt(trimmed, 10);
    const count = parsed != null && Number.isFinite(parsed) && parsed >= 1 ? parsed : null;
    // Reflect the normalized value back into the field.
    setExpectedSpeakerCount(count != null ? String(count) : '');
    try {
      await invoke('api_set_expected_speaker_count', { count });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save expected speaker count:', error);
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

  // Auto-summary toggle (specs/0029 WS7.3). Persistence is synchronous localStorage via
  // ConfigContext; the toast just matches the "Preference saved" feedback of siblings.
  const handleAutoSummaryToggle = (enabled: boolean) => {
    toggleIsAutoSummary(enabled);
    toast.success('Preference saved');
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

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
    }
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

  if (loading) {
    return (
      <div className="animate-pulse">
        <div className="h-4 bg-muted rounded w-1/4 mb-4"></div>
        <div className="h-8 bg-muted rounded mb-4"></div>
      </div>
    );
  }

  // What the single "Delete audio recordings" select shows for the stored
  // { auto_save, retention_days } pair. auto_save=false always reads as
  // "Immediately", regardless of any leftover retention value.
  const retentionChoice = retentionChoiceFromPreferences(preferences);

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-4">Recording Settings</h3>
        <p className="text-sm text-muted-foreground mb-6">
          Configure how your audio recordings are saved during meetings.
        </p>
      </div>

      {/* Delete audio recordings — ONE control combining the old "Save Audio
          Recordings" toggle and the retention window. The backend contract is
          unchanged: this maps onto { auto_save, retention_days } via
          lib/audio-retention.ts ("Immediately" = auto_save off; a day count or
          Never = auto_save on + that sweep window). Media files only: transcripts,
          notes, summaries, and metadata are kept, and meetings that haven't been
          transcribed yet are never swept. */}
      <div className="flex items-center justify-between gap-4 rounded-[3px] border border-border bg-card px-4 py-3">
        <div className="flex-1 pr-4">
          <div className="u-section-label">Delete audio recordings</div>
          <div className="text-sm text-muted-foreground">
            Choose how long the audio files of your meetings are kept.
            &quot;Immediately&quot; discards audio as soon as a recording stops. Only
            the audio is affected — transcripts, notes, and summaries are always
            kept, and meetings that haven&apos;t been transcribed yet are never
            deleted.
          </div>
        </div>
        <select
          value={retentionChoiceToSelectValue(retentionChoice)}
          onChange={(e) => void handleRetentionChange(e.target.value)}
          disabled={saving}
          aria-label="Delete audio recordings"
          className="rounded-md border border-input bg-background px-2 py-1 text-sm"
        >
          <option value="immediately">Immediately</option>
          <option value="7">After 7 days</option>
          <option value="30">After 30 days</option>
          <option value="90">After 90 days</option>
          {/* A custom value stored outside the presets still renders truthfully. */}
          {typeof retentionChoice === 'number' &&
            ![7, 30, 90].includes(retentionChoice) && (
              <option value={String(retentionChoice)}>After {retentionChoice} days</option>
            )}
          <option value="never">Never</option>
        </select>
      </div>

      {/* Folder Location - Only shown when audio is kept (auto_save on) */}
      {preferences.auto_save && (
        <div className="space-y-4">
          <div className="rounded-[3px] border border-border bg-muted p-4">
            <div className="font-medium mb-2">Save Location</div>
            <div className="text-sm text-muted-foreground mb-3 break-all">
              {preferences.save_folder || 'Default folder'}
            </div>
            <button
              onClick={handleOpenFolder}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-input rounded-md hover:bg-muted transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div>

          <div className="rounded-[3px] border border-border bg-muted p-4">
            <div className="text-sm text-foreground">
              <strong>File Format:</strong> {preferences.file_format.toUpperCase()} files
            </div>
            <div className="text-xs text-muted-foreground mt-1">
              Recordings are saved with timestamp: recording_YYYYMMDD_HHMMSS.{preferences.file_format}
            </div>
          </div>
        </div>
      )}

      {/* Info when audio is discarded immediately (auto_save off) */}
      {!preferences.auto_save && (
        <div className="rounded-[3px] border border-border bg-brand/10 p-4">
          <div className="text-sm text-foreground">
            Audio is deleted as soon as a recording stops — transcripts, notes, and
            summaries are still saved. Pick a time window (or &quot;Never&quot;) above
            to keep the audio files.
          </div>
        </div>
      )}

      {/* Live transcription toggle (specs/0029 WS7.2). Off = record-only mode:
          audio is still recorded (and the visualizers still work), but the STT
          stage is skipped to save CPU/battery; the meeting is transcribed later. */}
      <div className="rounded-[3px] border border-border bg-card px-4">
        <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
          <div className="flex-1 pr-4">
            <div className="text-sm font-medium text-foreground">Transcribe in real time during recording</div>
            <div className="text-sm text-muted-foreground">
              Show a live transcript while you record. Turning this off saves CPU and
              battery — audio is still recorded, and the meeting is transcribed later
              (automatically before a summary, or with &quot;Transcribe now&quot; on the
              meeting page).
            </div>
          </div>
          <Switch
            checked={preferences.live_transcription_enabled}
            onCheckedChange={handleLiveTranscriptionToggle}
            disabled={saving}
          />
        </div>

        {/* Low Power Mode on battery (low-power-mode spec). On by default (backend
            serde default true). Independent of the live-transcription toggle above —
            this one only kicks in on battery, and can be overridden per meeting while
            recording via the mode chip in the recording header. */}
        <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
          <div className="flex-1 pr-4">
            <div className="text-sm font-medium text-foreground">Low Power Mode on battery</div>
            <div className="text-sm text-muted-foreground">
              When on battery, record audio but defer transcription and summaries until
              you&apos;re back on power. You can override per meeting while recording.
            </div>
          </div>
          <Switch
            checked={preferences.low_power_on_battery}
            onCheckedChange={handleLowPowerToggle}
            disabled={saving}
          />
        </div>
      </div>

      {/* Transcription language (spec 0038 WS7.d) — the single global control for
          the transcription language preference (`set_language_preference`). This
          replaces the former in-recording "Language Settings" modal; there is no
          per-transcript/per-summary language. */}
      <div className="rounded-[3px] border border-border bg-card p-4">
        <LanguageSelection />
      </div>

      {/* Auto-summarize on meeting end (specs/0029 WS7.3) — same ConfigContext state as
          the Auto Summary toggle in Summary settings; one source of truth, two surfaces. */}
      <div className="rounded-[3px] border border-border bg-card px-4">
        <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
          <div className="flex-1 pr-4">
            <div className="text-sm font-medium text-foreground">Summarize automatically when a meeting ends</div>
            <div className="text-sm text-muted-foreground">
              Generate an AI summary as soon as a recording stops, using your configured
              summary model.
            </div>
          </div>
          <Switch
            checked={isAutoSummary}
            onCheckedChange={handleAutoSummaryToggle}
          />
        </div>

        {/* Recording Notification Toggle */}
        <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
          <div className="flex-1">
            <div className="u-section-label">Recording Start Notification</div>
            <div className="text-sm text-muted-foreground">
              Show reminder to inform participants when recording starts
            </div>
          </div>
          <Switch
            checked={showRecordingNotification}
            onCheckedChange={handleNotificationToggle}
          />
        </div>

        {/* Zoom Auto-Detect Toggle */}
        <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
          <div className="flex-1">
            <div className="u-section-label">Auto-detect Zoom meetings</div>
            <div className="text-sm text-muted-foreground">
              When a Zoom meeting starts, offer to record it
            </div>
          </div>
          <Switch
            checked={zoomAutoDetect}
            onCheckedChange={handleZoomAutoDetectToggle}
          />
        </div>
      </div>

      {/* Zoom mute gate (specs/0049) — opt-in, needs Accessibility permission. */}
      <ZoomMuteGateToggle />

      {/* Speaker diarization (specs/0010) — opt-in, default off. */}
      <div className="flex items-center justify-between gap-4 rounded-[3px] border border-border bg-card px-4 py-3">
        <div className="flex-1 pr-4">
          <div className="u-section-label">Speaker diarization</div>
          <div className="text-sm text-muted-foreground">
            Label who spoke in the transcript. Runs on-device after the meeting and
            downloads a small model (~35 MB) the first time.
          </div>
        </div>
        <Switch checked={diarizationEnabled} onCheckedChange={handleDiarizationToggle} />
      </div>

      {/* Live speaker labels (specs/0011, P3-B) — a sub-toggle of the diarization
          enable above. Only shown when diarization is on; renders indented and
          muted so it reads as subordinate. Default off; uses extra CPU. */}
      {diarizationEnabled && (
        <div className="ml-6 flex items-center justify-between gap-4 border-l-2 border-border bg-muted/50 rounded-r-[3px] p-4">
          <div className="flex-1 pr-4">
            <div className="font-medium">Label speakers live while recording</div>
            <div className="text-sm text-muted-foreground">
              Show provisional speaker labels on the live transcript as you record,
              instead of only after the meeting. Uses extra CPU.
            </div>
          </div>
          <Switch
            checked={liveDiarizationEnabled}
            onCheckedChange={handleLiveDiarizationToggle}
          />
        </div>
      )}

      {/* Expected speaker count override (specs/0011 accuracy gate). Only shown
          when diarization is on; the most reliable lever when automatic speaker
          detection over- or under-splits. Empty = automatic. */}
      {diarizationEnabled && (
        <div className="ml-6 flex items-center justify-between gap-4 border-l-2 border-border bg-muted/50 rounded-r-[3px] p-4">
          <div className="flex-1 pr-4">
            <div className="font-medium">Expected number of speakers</div>
            <div className="text-sm text-muted-foreground">
              Leave blank to detect automatically. If labels split one person into
              several (or merge several into one), set the exact number of people
              who spoke to force that many speakers.
            </div>
          </div>
          <input
            type="number"
            min={1}
            max={50}
            inputMode="numeric"
            placeholder="Auto"
            value={expectedSpeakerCount}
            onChange={(e) => setExpectedSpeakerCount(e.target.value)}
            onBlur={handleExpectedSpeakerCountCommit}
            className="w-20 rounded-md border border-input px-2 py-1 text-sm"
            aria-label="Expected number of speakers"
          />
        </div>
      )}

      {/* Voice identification / voiceprints (specs/0016 1c, ADR-0007). Only shown when
          diarization is on, since cross-meeting voice memory is built from diarized
          speakers. The global "store others" gate is OFF by default. */}
      {diarizationEnabled && (
        <div className="border-t pt-6">
          <h4 className="text-base font-medium text-foreground mb-1">Voice identification</h4>
          <p className="text-sm text-muted-foreground mb-4">
            Nixon can learn voices to recognize the same person across meetings. Voiceprints are
            stored only on this Mac and never leave your machine — they&apos;re never sent to any
            summary or AI provider.
          </p>

          <div className="rounded-[3px] border border-border bg-card px-4">
            {/* Global opt-in to store OTHER people's voiceprints — off by default. */}
            <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
              <div className="flex-1 pr-4">
                <div className="text-sm font-medium text-foreground">Store voiceprints for other people</div>
                <div className="text-sm text-muted-foreground">
                  Off by default. When on, Nixon remembers other people&apos;s voices to suggest names
                  automatically in future meetings. When off, names still suggest within a single
                  meeting, but no cross-meeting voice memory is kept for others. Voiceprints stay on
                  this Mac either way.
                </div>
              </div>
              <Switch
                checked={storeOthersVoiceprints}
                onCheckedChange={handleStoreOthersToggle}
              />
            </div>

            {/* Device-owner self-enroll — on by default. */}
            <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
              <div className="flex-1 pr-4">
                <div className="text-sm font-medium text-foreground">Recognize my own voice across meetings</div>
                <div className="text-sm text-muted-foreground">
                  Learns your voice (the microphone channel) so &quot;You&quot; is labeled reliably in
                  every meeting. Stored only on this Mac.
                </div>
              </div>
              <Switch
                checked={selfEnrollVoiceprint}
                onCheckedChange={handleSelfEnrollToggle}
              />
            </div>

            {/* Destructive: wipe the entire gallery. */}
            <div className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
              <div className="flex-1 pr-4">
                <div className="u-section-label">Clear all voiceprints</div>
                <div className="text-sm text-muted-foreground">
                  Delete every stored voice sample. People and their names are kept; Nixon just
                  re-learns voices from scratch.
                </div>
              </div>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setClearVoiceprintsOpen(true)}
              >
                Clear all
              </Button>
            </div>
          </div>
        </div>
      )}

      <ClearVoiceprintsDialog
        open={clearVoiceprintsOpen}
        onOpenChange={setClearVoiceprintsOpen}
      />

      {/* Device Preferences */}
      <div className="space-y-4">
        <div className="border-t pt-6">
          <h4 className="text-base font-medium text-foreground mb-4">Default Audio Devices</h4>
          <p className="text-sm text-muted-foreground mb-4">
            Set your preferred microphone and system audio devices for recording. These will be automatically selected when starting new recordings.
          </p>

          <div className="rounded-[3px] border border-border bg-muted p-4">
            <DeviceSelection
              selectedDevices={{
                micDevice: preferences.preferred_mic_device,
                systemDevice: preferences.preferred_system_device
              }}
              onDeviceChange={handleDeviceChange}
              disabled={saving}
            />
          </div>
        </div>
      </div>
    </div>
  );
}