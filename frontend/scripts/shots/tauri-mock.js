// tauri-mock.js — a browser-side stub of the Tauri v2 runtime, injected BEFORE the
// app boots (via CDP Page.addScriptToEvaluateOnNewDocument) so the Next app can run
// in plain headless Chrome for screenshots. Without this, the providers call
// `invoke(...)`, get no Tauri runtime, and WEDGE the renderer (no screenshot possible).
//
// Two things matter:
//  1) `window.__TAURI_INTERNALS__.invoke` must resolve (never reject) so providers don't
//     hang. Unknown commands return a render-safe default ([] for lists, {} for config).
//  2) The onboarding gate: the app renders BLANK until `get_onboarding_status` reports a
//     completed onboarding with a `model_status.parakeet` field — so we return that shape.
//
// Edit the FIXTURES below to change the sample data the screens render.
(function () {
  const ONBOARDING = {
    version: '1',
    completed: true,
    current_step: 99,
    model_status: { parakeet: 'downloaded', summary: 'downloaded', selected_summary_model: 'llama3' },
    last_updated: new Date().toISOString(),
  };

  const FIXTURES = {
    get_onboarding_status: ONBOARDING,
    api_get_onboarding_status: ONBOARDING,
    get_recording_state: { is_recording: false, is_paused: false, recording_duration: 0, active_duration: 0, status: 'idle' },
    api_get_meetings: [
      { id: 'm1', title: 'UI/UX Design Review', createdAt: new Date(Date.now() - 3600e3).toISOString(), durationSeconds: 3720 },
      { id: 'm2', title: 'Weekly 1:1 with Sarah', createdAt: new Date(Date.now() - 7200e3).toISOString(), durationSeconds: 1800 },
    ],
    api_get_day_agenda: [],
    get_calendar_access_status: 'notDetermined',
    api_get_calendar_access_status: 'notDetermined',
    api_get_meeting_notes: '',
    api_get_summary: { status: 'idle', blocks: [] },
  };

  // Render-safe default for any command not in FIXTURES.
  function fallback(cmd) {
    if (/config|status|state|settings|info|summary/i.test(cmd)) return {};
    return []; // iterable + .map-safe (the common cause of crashes is `.map` on null)
  }

  window.__TAURI_INTERNALS__ = {
    invoke: (cmd) => Promise.resolve(cmd in FIXTURES ? FIXTURES[cmd] : fallback(cmd)),
    transformCallback: () => Math.floor(Math.random() * 1e9),
    convertFileSrc: (p) => p,
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  };
})();
