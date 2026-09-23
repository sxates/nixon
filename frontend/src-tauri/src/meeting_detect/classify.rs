// Which call is live, from one poll's worth of facts (specs/0074 W6).
//
// Pure: the monitor samples the process list, the calendar and (optionally) a browser window
// title, and this decides. Everything here is unit-tested with fixture process lists.
//
// PROVISIONAL: the Teams and browser signals are the spec's hypothesis until the owner's
// spike (the `meeting_detect_spike` ignored test) shows which process owns the microphone
// in new Teams and whether browsers keep input running while muted. Zoom's signal is the
// proven one from specs/0008 and is unchanged.

use serde::Serialize;

/// The app a detected call is on. Serialised lowercase: the frontend's `PLATFORM_NAMES`
/// (`lib/meeting-detect.ts`) turns it into "Zoom call detected", "Teams call detected", …
///
/// The spec's `BrowserCall` (an uncorroborated browser microphone) is not a variant: the
/// owner's default is that it never prompts, so nothing would ever construct it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Zoom,
    Teams,
    Meet,
}

impl Platform {
    /// Does this call ending stop a recording? Only Zoom's end is trustworthy: Zoom tears
    /// its meeting helpers down on leave. Teams and browsers may release the microphone on
    /// mute, on a device switch or in a breakout, and stopping someone's recording on mute is
    /// far worse than not stopping it at hang-up. The frontend applies this rule; this is the
    /// same rule, stated where the platforms are defined.
    pub fn end_stops_recording(self) -> bool {
        matches!(self, Platform::Zoom)
    }
}

/// One Core Audio client process, as sampled from `kAudioHardwarePropertyProcessObjectList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioProc {
    pub pid: i32,
    /// Empty when Core Audio reports none (an unbundled binary).
    pub bundle_id: String,
    /// `kAudioProcessPropertyIsRunningInput`: the process is capturing from an input device.
    pub running_input: bool,
}

/// Microsoft Teams: new Teams (`MSTeams`) and classic. Matched as a bundle-id segment prefix,
/// so helpers such as `<prefix>.helper` count too.
const TEAMS_BUNDLE_PREFIXES: [&str; 2] = ["com.microsoft.teams2", "com.microsoft.teams"];

/// Browsers whose microphone may be a web call. `com.apple.WebKit.GPU` is Safari's media
/// process, shared by every WebKit app, which is one more reason a browser needs corroboration.
const BROWSER_BUNDLE_PREFIXES: [&str; 5] = [
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "company.thebrowser.Browser",
    "org.mozilla.firefox",
    "com.apple.WebKit.GPU",
];

/// A Google Meet tab's window title: "Meet - abc-defg-hij".
const MEET_TITLE_PREFIX: &str = "Meet - ";

/// `bundle_id` is `prefix` or `prefix.<anything>`, ignoring ASCII case.
fn bundle_matches(bundle_id: &str, prefix: &str) -> bool {
    let Some(head) = bundle_id.get(..prefix.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(prefix)
        && matches!(bundle_id.as_bytes().get(prefix.len()), None | Some(b'.'))
}

fn is_teams(bundle_id: &str) -> bool {
    TEAMS_BUNDLE_PREFIXES
        .iter()
        .any(|p| bundle_matches(bundle_id, p))
}

fn is_browser(bundle_id: &str) -> bool {
    BROWSER_BUNDLE_PREFIXES
        .iter()
        .any(|p| bundle_matches(bundle_id, p))
}

fn is_meet_title(title: &str) -> bool {
    title.trim_start().starts_with(MEET_TITLE_PREFIX)
}

/// Processes other than Nixon that are capturing from the microphone right now.
fn live_mics(procs: &[AudioProc], own_pid: i32) -> impl Iterator<Item = &AudioProc> {
    procs
        .iter()
        .filter(move |p| p.pid != own_pid && p.running_input)
}

/// Pids of browser processes (not Nixon) holding the microphone. The monitor reads the
/// calendar and the window title only when this is non-empty, so an idle poll costs one
/// Core Audio query and nothing else.
pub fn browser_mic_pids(procs: &[AudioProc], own_pid: i32) -> Vec<i32> {
    live_mics(procs, own_pid)
        .filter(|p| is_browser(&p.bundle_id))
        .map(|p| p.pid)
        .collect()
}

/// Which call is live, if any. Precedence: Zoom (its meeting helpers are the proven signal),
/// then Teams, then a browser call that something else corroborates.
///
/// - `zoom_helpers_present`: Zoom's `cpthost`/`aomhost` is running (specs/0008).
/// - `own_pid`: Nixon's pid; its own microphone never counts.
/// - `calendar_event_now`: a timed, non-hidden calendar event is in progress.
/// - `ax_title`: a browser window title, read only when Accessibility is ALREADY granted.
///
/// A browser microphone with neither a calendar event nor a Meet window title is `None`:
/// dictation sites and voice notes use the microphone too (owner default, 2026-09-22).
pub fn classify(
    zoom_helpers_present: bool,
    procs: &[AudioProc],
    own_pid: i32,
    calendar_event_now: bool,
    ax_title: Option<&str>,
) -> Option<Platform> {
    if zoom_helpers_present {
        return Some(Platform::Zoom);
    }
    let mut browser_mic = false;
    for p in live_mics(procs, own_pid) {
        if is_teams(&p.bundle_id) {
            return Some(Platform::Teams);
        }
        browser_mic |= is_browser(&p.bundle_id);
    }
    let corroborated = calendar_event_now || ax_title.is_some_and(is_meet_title);
    (browser_mic && corroborated).then_some(Platform::Meet)
}

/// Corroboration is needed to START a browser call, not to keep it: once a Meet call is
/// confirmed, it stays live while the browser holds the microphone, so a call that runs past
/// its calendar slot does not "end" at the slot's end time.
pub fn calendar_corroborates(calendar_event_now: bool, current: Option<Platform>) -> bool {
    calendar_event_now || current == Some(Platform::Meet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: i32 = 4242;

    fn proc(pid: i32, bundle_id: &str, running_input: bool) -> AudioProc {
        AudioProc {
            pid,
            bundle_id: bundle_id.to_string(),
            running_input,
        }
    }

    /// Background noise present on any Mac: output-only clients and idle inputs.
    fn idle_machine() -> Vec<AudioProc> {
        vec![
            proc(100, "com.apple.Music", false),
            proc(101, "com.apple.systemsounds", false),
            proc(102, "com.google.Chrome.helper", false),
            proc(OWN, "ai.vinyl.app", false),
        ]
    }

    fn with(mut procs: Vec<AudioProc>, extra: AudioProc) -> Vec<AudioProc> {
        procs.push(extra);
        procs
    }

    #[test]
    fn an_idle_machine_is_no_call() {
        assert_eq!(classify(false, &idle_machine(), OWN, true, None), None);
    }

    #[test]
    fn zoom_helpers_are_zoom_whatever_else_is_running() {
        let procs = with(idle_machine(), proc(200, "com.microsoft.teams2", true));
        assert_eq!(
            classify(true, &procs, OWN, false, None),
            Some(Platform::Zoom)
        );
        assert_eq!(
            classify(true, &idle_machine(), OWN, false, None),
            Some(Platform::Zoom)
        );
    }

    #[test]
    fn new_teams_on_the_microphone_is_teams() {
        let procs = with(idle_machine(), proc(200, "com.microsoft.teams2", true));
        assert_eq!(
            classify(false, &procs, OWN, false, None),
            Some(Platform::Teams)
        );
    }

    #[test]
    fn a_new_teams_helper_on_the_microphone_is_teams() {
        let procs = with(
            idle_machine(),
            proc(201, "com.microsoft.teams2.helper", true),
        );
        assert_eq!(
            classify(false, &procs, OWN, false, None),
            Some(Platform::Teams)
        );
    }

    #[test]
    fn classic_teams_on_the_microphone_is_teams() {
        let procs = with(idle_machine(), proc(200, "com.microsoft.teams", true));
        assert_eq!(
            classify(false, &procs, OWN, false, None),
            Some(Platform::Teams)
        );
    }

    #[test]
    fn teams_open_but_not_on_the_microphone_is_no_call() {
        let procs = with(idle_machine(), proc(200, "com.microsoft.teams2", false));
        assert_eq!(classify(false, &procs, OWN, true, None), None);
    }

    #[test]
    fn a_lookalike_bundle_id_is_not_teams() {
        let procs = with(idle_machine(), proc(200, "com.microsoft.teamsxyz", true));
        assert_eq!(classify(false, &procs, OWN, false, None), None);
    }

    #[test]
    fn chrome_on_the_microphone_during_a_calendar_event_is_meet() {
        let procs = with(idle_machine(), proc(300, "com.google.Chrome.helper", true));
        assert_eq!(
            classify(false, &procs, OWN, true, None),
            Some(Platform::Meet)
        );
    }

    #[test]
    fn chrome_on_the_microphone_with_no_calendar_event_is_no_call() {
        let procs = with(idle_machine(), proc(300, "com.google.Chrome.helper", true));
        assert_eq!(classify(false, &procs, OWN, false, None), None);
    }

    #[test]
    fn a_dismissed_event_does_not_corroborate() {
        // The monitor turns a hidden event into `calendar_event_now = false`
        // (`calendar::event_in_progress`, tested there); the classifier then stays quiet.
        let procs = with(idle_machine(), proc(300, "com.google.Chrome.helper", true));
        assert_eq!(classify(false, &procs, OWN, false, None), None);
    }

    #[test]
    fn a_meet_window_title_corroborates_without_an_event() {
        let procs = with(idle_machine(), proc(300, "com.google.Chrome.helper", true));
        assert_eq!(
            classify(false, &procs, OWN, false, Some("Meet - abc-defg-hij")),
            Some(Platform::Meet)
        );
        assert_eq!(
            classify(false, &procs, OWN, false, Some("Inbox - Gmail")),
            None
        );
    }

    #[test]
    fn every_listed_browser_counts_with_corroboration() {
        for bundle in [
            "com.google.Chrome.helper",
            "com.google.Chrome.canary",
            "com.microsoft.edgemac.helper",
            "company.thebrowser.browser.helper",
            "org.mozilla.firefox",
            "com.apple.WebKit.GPU",
        ] {
            let procs = with(idle_machine(), proc(300, bundle, true));
            assert_eq!(
                classify(false, &procs, OWN, true, None),
                Some(Platform::Meet),
                "{bundle}"
            );
            assert_eq!(classify(false, &procs, OWN, false, None), None, "{bundle}");
        }
    }

    #[test]
    fn a_calendar_event_alone_is_no_call() {
        assert_eq!(classify(false, &idle_machine(), OWN, true, None), None);
    }

    #[test]
    fn nixons_own_microphone_is_ignored() {
        // Nixon itself holding the microphone, even under a bundle id that would otherwise
        // match, is never a detected call.
        let procs = vec![
            proc(OWN, "com.microsoft.teams2", true),
            proc(OWN, "com.google.Chrome.helper", true),
        ];
        assert_eq!(classify(false, &procs, OWN, true, None), None);
        assert!(browser_mic_pids(&procs, OWN).is_empty());
    }

    #[test]
    fn browser_mic_pids_lists_only_capturing_browsers() {
        let procs = vec![
            proc(300, "com.google.Chrome.helper", true),
            proc(301, "com.google.Chrome.helper", false),
            proc(302, "com.microsoft.teams2", true),
            proc(303, "org.mozilla.firefox", true),
        ];
        assert_eq!(browser_mic_pids(&procs, OWN), vec![300, 303]);
    }

    #[test]
    fn a_confirmed_meet_call_outlives_its_calendar_slot() {
        assert!(calendar_corroborates(false, Some(Platform::Meet)));
        assert!(calendar_corroborates(true, None));
        assert!(!calendar_corroborates(false, None));
        assert!(!calendar_corroborates(false, Some(Platform::Teams)));
    }

    #[test]
    fn only_zoom_ending_stops_a_recording() {
        assert!(Platform::Zoom.end_stops_recording());
        assert!(!Platform::Teams.end_stops_recording());
        assert!(!Platform::Meet.end_stops_recording());
    }

    #[test]
    fn platforms_serialise_as_the_names_the_frontend_knows() {
        let names: Vec<String> = [Platform::Zoom, Platform::Teams, Platform::Meet]
            .iter()
            .map(|p| serde_json::to_string(p).unwrap())
            .collect();
        assert_eq!(names, ["\"zoom\"", "\"teams\"", "\"meet\""]);
    }
}
