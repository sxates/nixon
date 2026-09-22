import React, { useState, useEffect } from 'react';
import { Switch } from '@/components/ui/switch';
import ZoomMuteGateToggle from '@/components/ZoomMuteGateToggle';
import { invoke } from '@tauri-apps/api/core';
import { SaveLocationRow } from '@/components/SaveLocationRow';
import {
  applyRetentionChoice,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
} from '@/lib/audio-retention';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { toast } from 'sonner';
import { patchRecordingPreferences } from '@/lib/recording-preferences';
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
  /** The user picked `save_folder` by hand rather than inheriting the probe's default
   *  (2026-09-21). Only the debug build reads it — see `repoint_dev_root` in
   *  `audio/recording_preferences.rs`. */
  save_folder_user_chosen?: boolean;
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
  const [showRecordingNotification, setShowRecordingNotification] = useState(true);
  const [zoomAutoDetect, setZoomAutoDetect] = useState(true);
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
      await patchRecordingPreferences<RecordingPreferences>(newPreferences);
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
      await patchRecordingPreferences<RecordingPreferences>(newPreferences);
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
      await patchRecordingPreferences<RecordingPreferences>(newPreferences);
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
                disabled={loading}
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
                disabled={loading}
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
          {/* Zoom mute gate (specs/0049) — opt-in, needs Accessibility permission. One of
              the things Nixon does while recording, so it sits with them (specs/0067). */}
          <ZoomMuteGateToggle />
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
              /* The app's Select, not a bare <select> — this was the one control still
                 wearing the browser's chrome (specs/0067). */
              <Select
                value={retentionChoiceToSelectValue(retentionChoice)}
                onValueChange={(value) => void handleRetentionChange(value)}
                disabled={loading}
              >
                <SelectTrigger id="audio-retention" className="w-48">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="immediately">Immediately</SelectItem>
                  <SelectItem value="7">After 7 days</SelectItem>
                  <SelectItem value="30">After 30 days</SelectItem>
                  <SelectItem value="90">After 90 days</SelectItem>
                  {/* A custom value stored outside the presets still renders truthfully. */}
                  {typeof retentionChoice === 'number' &&
                    ![7, 30, 90].includes(retentionChoice) && (
                      <SelectItem value={String(retentionChoice)}>
                        After {retentionChoice} days
                      </SelectItem>
                    )}
                  <SelectItem value="never">Never</SelectItem>
                </SelectContent>
              </Select>
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

    </div>
  );
}
