#!/usr/bin/env node
/**
 * run-nixon driver — drive the Nixon frontend in a real browser and screenshot it.
 *
 * Nixon is a Tauri 2 + Next.js desktop app. The native window needs macOS
 * screen-recording/mic permissions, Metal, and downloaded models — not automatable
 * headless. But the *frontend* (the React UI where most PRs land) can be rendered in
 * plain Chrome IF we stub the Tauri `invoke` IPC bridge it calls on bootstrap. This
 * driver does exactly that: it injects a `window.__TAURI_INTERNALS__` shim returning
 * mock data per command, loads a route, and screenshots it.
 *
 * Drives the system Google Chrome via puppeteer-core (installed as a frontend devDep —
 * no Chromium download). Resolve puppeteer-core from frontend/node_modules regardless
 * of where this script lives.
 *
 * Usage (from the repo root or anywhere):
 *   node .claude/skills/run-nixon/driver.mjs --route /people --out /tmp/people.png
 *   node .claude/skills/run-nixon/driver.mjs --route /settings --out /tmp/settings.png
 * Options: --port (default 3119), --delay ms after load (default 2500),
 *          --width/--height viewport.
 *
 * Prereq: a dev server on --port (see SKILL.md: `pnpm exec next dev -p 3119`).
 */
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

// puppeteer-core lives in frontend/node_modules; this file is in .claude/skills/run-nixon/.
const require = createRequire(new URL('../../../frontend/package.json', import.meta.url));
const puppeteer = require('puppeteer-core');

const CHROME =
  process.env.CHROME_PATH ||
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function arg(name, def) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : def;
}

// All values for a repeatable flag, in order (e.g. several `--click` to chain a
// multi-step interaction: open a tab, then click something inside it).
function args(name) {
  const out = [];
  for (let i = 0; i < process.argv.length - 1; i++) {
    if (process.argv[i] === `--${name}`) out.push(process.argv[i + 1]);
  }
  return out;
}

const route = arg('route', '/');
const out = arg('out', '/tmp/nixon-shot.png');
const port = arg('port', '3119');
const delay = parseInt(arg('delay', '2500'), 10);
const width = parseInt(arg('width', '1440'), 10);
const height = parseInt(arg('height', '900'), 10);

// Mock data for the commands the shell + common pages call on mount. Anything not
// listed falls through to a sensible default (null) so a missing mock can't crash a
// provider. Add commands here as you drive deeper pages.
const PEOPLE = [
  { id: 'p1', displayName: 'Ada Example', role: 'Founder', email: 'brian@example.com', notes: null, voiceprintOptOut: false, createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:00Z' },
  { id: 'p2', displayName: 'Jordan Lee', role: 'Engineer', email: 'jordan.lee@acme.io', notes: null, voiceprintOptOut: true, createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:00Z' },
  { id: 'p3', displayName: 'Priya Patel', role: 'Design', email: 'priya@acme.io', notes: null, voiceprintOptOut: false, createdAt: '2026-01-01T00:00:00Z', updatedAt: '2026-01-01T00:00:00Z' },
];

// A long-ish transcript so meeting-details scrolls (lets you see sticky tab nav, WS1.1).
const TRANSCRIPT = Array.from({ length: 40 }, (_, i) => ({
  id: `t${i}`,
  meeting_id: 'm1',
  transcript: `This is transcript line number ${i + 1}, said by one of the speakers in the meeting.`,
  text: `This is transcript line number ${i + 1}, said by one of the speakers in the meeting.`,
  timestamp: new Date(1735732800000 + i * 8000).toISOString(),
  audio_start_time: i * 8,
  audio_end_time: i * 8 + 7,
  duration: 7,
  speaker: i % 2 === 0 ? 'local' : 'spk_0',
  speaker_name: i % 2 === 0 ? 'You' : 'Speaker 1',
}));

// Today view timeline (specs/0036 WS7): a spread of items around the current time so
// the hour grid, now-line, and past/now/upcoming states all render. Times are relative
// to "now" and include `seriesKey` on recurring calendar events.
const AGENDA_NOW = Date.now();
const atOffset = (minutes) => new Date(AGENDA_NOW + minutes * 60_000).toISOString();
const agendaStatus = (over = {}) => ({
  recorded: false,
  transcribed: false,
  summarized: false,
  speakersIdentified: false,
  ...over,
});
const TODAY_AGENDA = [
  // Back-to-back 15-min pair (adjacent, non-overlapping): must render STACKED
  // full-width, not staggered into left/right lanes (specs/0041 WS6).
  {
    id: 'evt-adjacent-a',
    title: 'Vendor Sync',
    startTime: atOffset(60),
    endTime: atOffset(75),
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 3,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-vendor',
  },
  {
    id: 'evt-adjacent-b',
    title: 'Budget Check-in',
    startTime: atOffset(75),
    endTime: atOffset(90),
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 2,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-budget',
  },
  {
    id: 'rec-earlier',
    title: 'Morning Standup',
    startTime: atOffset(-150),
    endTime: atOffset(-135),
    source: 'recording',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 4,
    meetingId: 'm-earlier',
    status: agendaStatus({ recorded: true, transcribed: true, summarized: true }),
    dismissed: false,
    seriesKey: null,
  },
  {
    id: 'evt-missed',
    title: 'Design Critique',
    startTime: atOffset(-70),
    endTime: atOffset(-25),
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 6,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-critique',
  },
  {
    id: 'evt-now',
    title: 'Leadership Review',
    startTime: atOffset(-10),
    endTime: atOffset(35),
    source: 'calendar',
    zoomUrl: 'https://zoom.us/j/9876543210',
    attendees: [],
    attendeeCount: 5,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-leadership',
  },
  {
    id: 'evt-upcoming-1',
    title: 'UX Weekly Sync',
    startTime: atOffset(75),
    endTime: atOffset(120),
    source: 'calendar',
    zoomUrl: 'https://meet.google.com/abc-defg-hij',
    attendees: [],
    attendeeCount: 8,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-ux-weekly',
  },
  {
    id: 'evt-upcoming-2',
    title: '1:1 with Jordan',
    startTime: atOffset(150),
    endTime: atOffset(180),
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 2,
    meetingId: null,
    status: agendaStatus(),
    dismissed: false,
    seriesKey: 'ical-1on1-jordan',
  },
];

// Recorded meetings for the /meetings month grid (specs/0054 W3). Built around the
// real current month so the "today" pip and the default cursor line up.
const MONTH_NOW = new Date();
const onDay = (day, hour) =>
  new Date(MONTH_NOW.getFullYear(), MONTH_NOW.getMonth(), day, hour, 0, 0, 0).toISOString();
const MONTH_MEETINGS = [
  { id: 'mm1', title: 'Weekly Sync', createdAt: onDay(3, 10), durationSeconds: 1800, hasSummary: true },
  { id: 'mm2', title: 'Design review: onboarding', createdAt: onDay(3, 14), durationSeconds: 2700, hasSummary: false },
  { id: 'mm3', title: '1:1 with Jordan', createdAt: onDay(9, 9), durationSeconds: 1500, hasSummary: true },
  { id: 'mm4', title: 'Roadmap planning', createdAt: onDay(12, 11), durationSeconds: 3600, hasSummary: true },
  { id: 'mm5', title: 'Vendor call - Acme', createdAt: onDay(12, 13), durationSeconds: 1200, hasSummary: false },
  { id: 'mm6', title: 'Standup', createdAt: onDay(12, 16), durationSeconds: 600, hasSummary: true },
  { id: 'mm7', title: 'Retro', createdAt: onDay(12, 17), durationSeconds: 900, hasSummary: false },
  { id: 'mm8', title: 'Customer interview', createdAt: onDay(18, 15), durationSeconds: 2400, hasSummary: true },
  { id: 'mm9', title: 'Board prep', createdAt: onDay(24, 8), durationSeconds: 3000, hasSummary: true },
];

// Prep view for the record screen's Prep drawer (specs/0054 W4).
const PREP_VIEW = {
  meetingId: 'm1',
  origin: 'recorded',
  title: 'Weekly Sync',
  briefStatus: 'ready',
  briefMarkdown:
    '## Where we left off\n\nLast week the team agreed to cut the export feature from the 1.18 scope and revisit it after the calendar work lands.\n\n## Open threads\n\n- Pricing page copy is still unreviewed\n- Jordan owed a decision on the vendor contract',
  briefSources: [{ meetingId: 'm0', title: 'Weekly Sync', date: '2026-08-18T17:00:00Z' }],
  openItems: [
    { id: 'ai1', description: 'Send the updated deck to Jordan', assignee: 'You', dueDate: null, status: 'open' },
    { id: 'ai2', description: 'Review pricing page copy', assignee: 'You', dueDate: null, status: 'open' },
    { id: 'ai3', description: 'Confirm vendor contract terms', assignee: 'Jordan', dueDate: null, status: 'open' },
  ],
  prepNotesMarkdown: '- Ask about Q3 headcount\n- Confirm launch date',
  prepNotesJson: null,
  linkedMeetings: [],
};

// Recorded meetings for the /meetings LIST view (specs/0054 W3). Spread over today,
// yesterday and two older days so the day-card grouping is visible.
const daysAgoAt = (days, hour, min = 0) => {
  const d = new Date(MONTH_NOW);
  d.setDate(d.getDate() - days);
  d.setHours(hour, min, 0, 0);
  return d.toISOString();
};
const LIST_MEETINGS = [
  { id: 'lm1', title: 'Standup', createdAt: daysAgoAt(0, 9, 15), durationSeconds: 720, attendees: [], attendeeCount: 0 },
  { id: 'lm2', title: 'Roadmap planning', createdAt: daysAgoAt(0, 11), durationSeconds: 3600, attendees: PEOPLE.slice(0, 3).map((p, i) => ({ name: p.displayName, email: p.email, isCurrentUser: i === 0, responseStatus: 'accepted', isOrganizer: i === 0 })), attendeeCount: 5 },
  { id: 'lm3', title: 'Design review: onboarding', createdAt: daysAgoAt(1, 14), durationSeconds: 2700, attendees: [], attendeeCount: 0 },
  { id: 'lm4', title: '1:1 with Jordan', createdAt: daysAgoAt(1, 16, 30), durationSeconds: 1500, attendees: [], attendeeCount: 2 },
  { id: 'lm5', title: 'Customer interview - Northwind', createdAt: daysAgoAt(4, 10), durationSeconds: 2400, attendees: [], attendeeCount: 0 },
  { id: 'lm6', title: 'Vendor call - Acme', createdAt: daysAgoAt(9, 13), durationSeconds: 1200, attendees: [], attendeeCount: 0 },
];

const MOCKS = {
  api_list_people: PEOPLE,
  // Today view (specs/0036 WS7).
  api_get_calendar_access_status: 'authorized',
  api_ensure_scheduled_meeting: 'meeting-scheduled-1',
  // meeting-details (route /meeting-details?id=m1)
  api_get_meeting_metadata: { id: 'm1', title: 'Weekly Sync', created_at: '2026-01-01T17:00:00Z', updated_at: '2026-01-01T18:00:00Z', folder_path: '/recordings/weekly-sync', origin: 'recorded', calendar_event_id: null },
  api_get_meeting_transcripts: { transcripts: TRANSCRIPT, has_more: false, total_count: TRANSCRIPT.length },
  api_get_summary: { status: 'idle', data: null },
  // Two diarized speakers so the SpeakerLegend renders (WS2.x verification).
  api_get_meeting_speakers: [
    { speakerKey: 'local', displayName: 'You', isLocal: true, email: null },
    { speakerKey: 'spk_0', displayName: 'Speaker 1', isLocal: false, email: null },
  ],
  api_get_meeting_notes: null,
  // 12 participants so the WS3.1 cap-at-10 + "+N more" expander is exercised.
  api_get_meeting_participants: Array.from({ length: 12 }, (_, i) => ({
    personId: `pp${i}`,
    displayName: `Participant ${i + 1}`,
    email: `participant${i + 1}@example.com`,
    role: i % 2 === 0 ? 'Engineer' : 'Design',
    source: 'calendar',
  })),
  api_get_meeting_attendees: { attendees: [], suggestion: null },
  api_get_speaker_suggestions: [],
  api_list_templates: [{ id: 'standard_meeting', name: 'Standard Meeting', description: '' }],
  api_get_custom_openai_config: {},
  builtin_ai_get_model_info: { ready: true },
  api_get_person_voiceprint_count: 0,
  api_get_voiceprint_settings: { storeOthersVoiceprints: false },
  api_get_day_agenda: TODAY_AGENDA,
  api_get_upcoming_meetings: [],
  api_get_meetings: LIST_MEETINGS,
  // specs/0054 W3 + W4.
  api_get_meetings_in_range: MONTH_MEETINGS,
  api_get_prep: PREP_VIEW,
  get_calendar_access_status: 'notDetermined',
  api_get_model_config: {},
  // RecordingStateContext polls this and reads `.is_recording` — must be a full object.
  get_recording_state: { is_recording: false, is_paused: false, is_processing: false, is_saving: false, status: 'idle' },
  is_recording: false,
  get_recording_meeting_name: null,
  // ConfigProvider (mounted in the layout) — lists must be iterable, paths are strings.
  get_ollama_models: [],
  get_notification_settings: {
    recording_notifications: true,
    time_based_reminders: false,
    meeting_reminders: false,
    respect_do_not_disturb: true,
    notification_sound: true,
    system_permission_granted: true,
    consent_given: true,
    manual_dnd_mode: false,
    notification_preferences: {
      show_recording_started: true,
      show_recording_stopped: true,
      show_recording_paused: false,
      show_recording_resumed: false,
      show_transcription_complete: true,
      show_meeting_reminders: false,
      show_system_errors: true,
      meeting_reminder_minutes: [],
    },
  },
  // Settings → General/Recordings tabs.
  api_get_owner_emails: ['brian@example.com'],
  api_google_calendar_status: {
    configured: true,
    connected: false,
    email: null,
    lastSyncedAt: null,
    calendars: [],
  },
  get_recording_preferences: {
    save_folder: '/Users/you/Movies/nixon-recordings',
    auto_save: true,
    file_format: 'mp4',
    preferred_mic_device: null,
    preferred_system_device: null,
    retention_days: 30,
    live_transcription_enabled: true,
  },
  get_audio_devices: [
    { name: 'MacBook Pro Microphone', device_type: 'Input', is_default: true },
    { name: 'System Audio', device_type: 'Output', is_default: true },
  ],
  start_audio_level_monitoring: null,
  stop_audio_level_monitoring: null,
  get_audio_backend_info: [],
  get_current_audio_backend: 'core-audio',
  api_get_zoom_auto_detect: true,
  api_get_diarization_enabled: false,
  api_get_live_diarization_enabled: false,
  api_get_expected_speaker_count: null,
  api_get_transcript_config: { provider: 'localWhisper', model: 'large-v3' },
  get_database_directory: '',
  whisper_get_models_directory: '',
  get_default_recordings_folder_path: '',
  api_get_api_key: '',
  // Onboarding gate: layout.tsx renders the welcome flow unless this reports completed.
  get_onboarding_status: { completed: true },
  check_first_launch: false,
  check_default_legacy_database: null,
  builtin_ai_get_recommended_model: '',
  builtin_ai_is_model_ready: true,
};

async function main() {
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-first-run', '--no-default-browser-check', '--disable-gpu'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width, height });

  // Inject the Tauri IPC shim BEFORE any app script runs.
  await page.evaluateOnNewDocument((mocks) => {
    const noop = () => {};
    const cb = (cb) => {
      const id = Math.floor(Math.random() * 1e9);
      // @ts-ignore
      window[`_${id}`] = typeof cb === 'function' ? cb : noop;
      return id;
    };
    // Default: resolve null. Event-plugin calls resolve an unlisten/registration id.
    const invoke = (cmd, args) => {
      if (cmd in mocks) return Promise.resolve(mocks[cmd]);
      if (typeof cmd === 'string' && cmd.startsWith('plugin:event|')) return Promise.resolve(1);
      return Promise.resolve(null);
    };
    // @ts-ignore — minimal surface @tauri-apps/api v2 reads.
    window.__TAURI_INTERNALS__ = {
      invoke,
      transformCallback: cb,
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: noop };
    window.isTauri = true;
  }, MOCKS);

  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console.error: ${m.text()}`);
  });

  const url = `http://localhost:${port}${route}`;
  process.stderr.write(`navigating ${url}\n`);
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await new Promise((r) => setTimeout(r, delay)); // let client render settle

  // Optional interaction: one or more --click <selector> in order (e.g. open a tab,
  // then click something that only renders once that tab is visible). `text=` matches
  // the first clickable element whose trimmed text equals the string.
  for (const clickSel of args('click')) {
    if (clickSel.startsWith('text=')) {
      const want = clickSel.slice(5);
      const clicked = await page.evaluate((want) => {
        const el = Array.from(document.querySelectorAll('button, a, [role="tab"], [role="menuitem"]'))
          .find((e) => (e.textContent || '').trim() === want);
        if (el) { el.scrollIntoView(); el.click(); return true; }
        return false;
      }, want);
      if (!clicked) process.stderr.write(`no element with text "${want}"\n`);
    } else {
      await page.waitForSelector(clickSel, { timeout: 5000 });
      await page.click(clickSel);
    }
    await new Promise((r) => setTimeout(r, 500));
  }

  // Optional interaction: --type <text> into --selector (default first <input>),
  // then re-settle. Used to drive e.g. the People search box (WS3.3).
  const typeText = arg('type', null);
  if (typeText != null) {
    const selector = arg('selector', 'input');
    await page.waitForSelector(selector, { timeout: 5000 });
    await page.click(selector);
    await page.type(selector, typeText, { delay: 20 });
    await new Promise((r) => setTimeout(r, 600));
  }

  // Optional: --scroll <px> scrolls the tallest scrollable element (the app uses an
  // inner overflow container, not the window). Used to test sticky headers (WS1.1).
  const scrollPx = arg('scroll', null);
  if (scrollPx != null) {
    await page.evaluate((px) => {
      const els = Array.from(document.querySelectorAll('*')).filter(
        (e) => e.scrollHeight > e.clientHeight + 20 && getComputedStyle(e).overflowY !== 'visible',
      );
      els.sort((a, b) => b.scrollHeight - a.scrollHeight);
      (els[0] || document.scrollingElement)?.scrollBy(0, Number(px));
    }, scrollPx);
    await new Promise((r) => setTimeout(r, 400));
  }

  await page.screenshot({ path: out, fullPage: false });

  const bodyText = (await page.evaluate(() => document.body?.innerText || '')).slice(0, 600);
  await browser.close();

  process.stdout.write(`\nscreenshot: ${out}\n`);
  process.stdout.write(`--- visible text (first 600 chars) ---\n${bodyText}\n`);
  if (errors.length) {
    process.stdout.write(`--- page/console errors (${errors.length}) ---\n${errors.slice(0, 12).join('\n')}\n`);
  }
}

main().catch((e) => {
  process.stderr.write(`driver failed: ${e?.stack || e}\n`);
  process.exit(1);
});
