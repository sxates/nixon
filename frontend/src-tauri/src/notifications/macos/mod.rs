// OS notifications that macOS actually delivers (specs/0068).
//
// Why this module exists at all: Nixon has had an OS-notification path since specs/0008 and
// none of it could ever have worked. `tauri-plugin-notification` registers exactly three
// commands on desktop — `notify`, `request_permission`, `is_permission_granted` — so the
// action-type registration and the `onAction` listener that carried the Record button are
// mobile-only and threw on every macOS call (swallowed by a `try/catch` in
// `lib/osNotification.ts`, which is why it looked fine); `request_permission` returns
// `Granted` without asking macOS, so the system never had a reason to show its dialog; and
// delivery ran through notify-rust on `NSUserNotificationCenter`, deprecated in 10.14 and
// nil outside an app bundle on macOS 26.
//
// So Nixon talks to `UNUserNotificationCenter` itself. That is also the only way to get the
// Join / Join & Record buttons, because action buttons live on `UNNotificationCategory`.
//
// # The bundle gate — read this before adding an entry point
//
// `+[UNUserNotificationCenter currentNotificationCenter]` does not fail politely outside an
// `.app`. It raises `NSInternalInconsistencyException` ("bundleProxyForCurrentProcess is
// nil"), an uncaught Objective-C exception, which aborts the process. `tauri dev` runs the
// bare binary out of `target/debug`, and so does `cargo test`. Every public function here
// therefore checks [`capability`] before touching the framework — a miss is not a missing
// banner, it is a crash on launch. `bundle_gate_holds` covers exactly that, and it is
// meaningful precisely because the test binary is unbundled.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
mod authorize;
#[cfg(target_os = "macos")]
mod delegate;
#[cfg(target_os = "macos")]
mod deliver;

/// Category carrying the meeting action. Registered at setup; referenced by id on every
/// alert `CalendarAlerts` sends.
///
/// **One action per category, deliberately.** macOS 26 renders a lone notification action
/// as a visible button and collapses two or more behind an "Options" menu — measured, not
/// assumed, with a throwaway app under a fresh bundle id: the same banner showed "Join" and
/// "Join & Record" under Options, and showed a "Join & Record" button when the category
/// carried only that one.
///
/// So the two meeting alerts carry different single actions instead of one alert carrying
/// two: at T-5 the useful thing is [`CATEGORY_PREP`] ("Prep"), and at T-0 it is this one
/// ("Join & Record"). Which is arguably the better design anyway — each alert offers the
/// thing you would actually do at that moment.
///
/// (The notification *style* — temporary vs persistent — makes no difference to this; both
/// were tried. There is no Info.plist key or category option that changes it either.)
pub const CATEGORY_MEETING: &str = "nixon.meeting";
/// Category for the five-minute warning. Its button opens the meeting's Prep tab — at
/// T-5 you are not joining yet, you are deciding what this meeting is for.
pub const CATEGORY_PREP: &str = "nixon.prep";
/// Category for "a call started — record it?" (`MeetingAutoDetect`), and for a meeting
/// starting with no join link. One button: macOS already gives dismissal for free, so an
/// "Ignore" button would only take up room.
pub const CATEGORY_RECORD: &str = "nixon.record";
/// Category with no buttons — the test banner and anything purely informational.
pub const CATEGORY_PLAIN: &str = "nixon.plain";

/// Open the meeting and start a Nixon recording bound to it.
pub const ACTION_JOIN_AND_RECORD: &str = "join_and_record";
/// Start recording the call that was detected.
pub const ACTION_RECORD: &str = "record";
/// Open the meeting's Prep tab.
pub const ACTION_PREP: &str = "prep";
/// Normalized identifier for "tapped the banner itself" — Apple sends
/// `UNNotificationDefaultActionIdentifier`, which is not a name the frontend should know.
pub const ACTION_OPEN: &str = "open";

/// Rust → frontend event emitted when someone presses a button or taps a banner.
pub const EVENT_ACTION: &str = "notification-action";

/// Whether this build can show an OS notification, and if not, why — in words the Settings
/// row prints verbatim rather than offering a button that cannot work.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub supported: bool,
    pub reason: Option<String>,
}

impl Capability {
    fn supported() -> Self {
        Self {
            supported: true,
            reason: None,
        }
    }

    fn unsupported(reason: &str) -> Self {
        Self {
            supported: false,
            reason: Some(reason.to_string()),
        }
    }
}

/// One notification to deliver.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverRequest {
    /// Caller-chosen id. Echoed back on the action event so the frontend can route the
    /// press to the right callback, and used to replace an already-delivered banner.
    pub id: String,
    pub title: String,
    pub body: String,
    /// [`CATEGORY_MEETING`] or [`CATEGORY_PLAIN`]; anything else is delivered without
    /// buttons, since macOS drops an unregistered category silently.
    #[serde(default)]
    pub category: Option<String>,
    /// Opaque string map handed back on the action event (the meeting id, its join URL).
    ///
    /// Deliberately kept on the Rust side, keyed by `id`, rather than put in the
    /// notification's `userInfo`: it keeps the Objective-C surface down to title, body and
    /// category, and nothing here needs to outlive the running app.
    #[serde(default)]
    pub user_info: HashMap<String, String>,
    /// Take this notification back down — off the screen and out of Notification Center —
    /// after this many milliseconds. `None` leaves it until the user dismisses it.
    ///
    /// **Why this exists instead of a style flag.** Owner feedback 2026-09-21: "Is it
    /// possible to control which OS notifications are transient, and which ones are
    /// persistent? The 'meeting starts now - join & record' should be persistent, but all
    /// others transient." macOS offers no such control. Banner-vs-Alert is one app-wide
    /// toggle in System Settings, `UNNotificationInterruptionLevel::TimeSensitive` needs an
    /// Apple-granted entitlement Nixon does not have, and specs/0068 already established
    /// (see [`CATEGORY_MEETING`]) that no Info.plist key or category option changes the
    /// style either — both were tried.
    ///
    /// So the app is set to **Alerts**, where everything persists, and Nixon removes the
    /// ones that should not have. Inverting the problem is the only lever the platform
    /// actually gives us.
    ///
    /// Left unset, [`default_auto_dismiss_ms`] decides from the category — which is the
    /// normal case, so no caller has to remember.
    #[serde(default)]
    pub auto_dismiss_ms: Option<u64>,
    /// Have macOS deliver this at an instant (epoch ms) instead of now (specs/0075 W2).
    ///
    /// The T-0 "starting now" banner is scheduled this way at T-5: the webview's 60s poll is
    /// throttled when Nixon is in the background, and macOS keeps its own clock. An instant
    /// less than [`SCHEDULE_MIN_LEAD_MS`] away (or already past) delivers now. Re-sending the
    /// same `id` replaces the pending request, and [`cancel_pending`] withdraws it.
    #[serde(default)]
    pub deliver_at_ms: Option<i64>,
}

/// How far in the future `deliver_at_ms` must be before it is worth a trigger. Anything
/// closer is delivered now — a one-second trigger buys nothing and a zero or negative
/// interval makes `UNTimeIntervalNotificationTrigger` raise.
pub const SCHEDULE_MIN_LEAD_MS: i64 = 1_000;

/// Seconds until a scheduled delivery, or `None` for "deliver now". Pure, so the
/// schedule-vs-now decision is testable without a notification centre.
pub fn schedule_delay_secs(deliver_at_ms: Option<i64>, now_ms: i64) -> Option<f64> {
    let lead_ms = deliver_at_ms?.checked_sub(now_ms)?;
    (lead_ms > SCHEDULE_MIN_LEAD_MS).then(|| lead_ms as f64 / 1000.0)
}

/// The auto-dismiss delay this delivery should get: the caller's explicit value, else the
/// category's default — but never for a scheduled request. That timer runs in Nixon from the
/// moment of the call, so on a banner macOS will show minutes later it would fire first and
/// remove the still-pending request (`remove` clears pending ones too), dropping the banner.
/// Only the start categories are scheduled today and they persist anyway; this keeps it so if
/// a transient category is ever scheduled.
pub fn effective_auto_dismiss_ms(request: &DeliverRequest, scheduled: bool) -> Option<u64> {
    if scheduled {
        return None;
    }
    request
        .auto_dismiss_ms
        .or_else(|| default_auto_dismiss_ms(request.category.as_deref()))
}

/// How long a banner of this category should last before Nixon takes it back down.
///
/// Exactly one kind of Nixon notification survives until dismissed: the one telling you a
/// meeting has started, because that is the single moment where missing the banner means
/// missing the recording. Everything else goes on its own.
///
/// A constant rather than a setting: the user already has the one setting that matters
/// (Alerts vs Banners, in System Settings), and a second control for "which of my alerts
/// are really alerts" would be asking them to do the app's job.
///
/// Pure, so the policy is unit-testable without a notification centre.
pub fn default_auto_dismiss_ms(category: Option<&str>) -> Option<u64> {
    match category {
        // "Starting now — Join & Record", and its link-less twin. Stays put.
        Some(CATEGORY_MEETING) | Some(CATEGORY_RECORD) => None,
        // The T-5 warning: if you missed it, the T-0 alert is coming and reuses the same
        // notification id, so it replaces this one anyway.
        Some(CATEGORY_PREP) => Some(TRANSIENT_MS),
        // Recording started/stopped, and anything else.
        _ => Some(TRANSIENT_MS),
    }
}

/// How long a transient banner stays. Long enough to read a title and a line of body,
/// short enough that a stack of them never accumulates.
pub const TRANSIENT_MS: u64 = 8_000;

/// What the user did with a banner.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPayload {
    /// [`ACTION_JOIN`], [`ACTION_JOIN_AND_RECORD`] or [`ACTION_OPEN`].
    pub action_id: String,
    pub notification_id: String,
    pub user_info: HashMap<String, String>,
}

/// Can this build deliver an OS notification?
pub fn capability() -> Capability {
    #[cfg(target_os = "macos")]
    {
        match bundle_identifier() {
            Some(_) => Capability::supported(),
            None => Capability::unsupported(
                "The dev build runs outside an .app bundle, where macOS will not hand out a \
                 notification centre. Notifications work in the installed app.",
            ),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Capability::unsupported("OS notifications are implemented on macOS only.")
    }
}

/// The bundle identifier of the running process, or `None` when it has no bundle — which is
/// the exact condition under which the UserNotifications framework aborts.
#[cfg(target_os = "macos")]
fn bundle_identifier() -> Option<String> {
    use objc2_foundation::NSBundle;

    // `-[NSBundle bundleIdentifier]` returns nil for a bundle-less process rather than
    // raising — which is precisely the signal this gate needs.
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}

/// Current authorization, as one of `not_determined` / `denied` / `authorized` /
/// `provisional` / `unavailable`. Never prompts.
pub async fn authorization_status() -> String {
    if !capability().supported {
        return "unavailable".to_string();
    }

    #[cfg(target_os = "macos")]
    {
        authorize::status().await
    }

    #[cfg(not(target_os = "macos"))]
    {
        "unavailable".to_string()
    }
}

/// Ask macOS for permission. Shows the system dialog the first time and only the first
/// time: once answered, the stored answer is returned without a prompt.
pub async fn request_authorization() -> Result<bool> {
    let capability = capability();
    if !capability.supported {
        return Err(anyhow::anyhow!(
            "{}",
            capability
                .reason
                .unwrap_or_else(|| "Notifications are unavailable.".to_string())
        ));
    }

    #[cfg(target_os = "macos")]
    {
        authorize::request().await
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(false)
    }
}

/// Deliver a notification — now, or at `deliver_at_ms` when that is in the future.
pub fn deliver(request: DeliverRequest) -> Result<()> {
    let capability = capability();
    if !capability.supported {
        return Err(anyhow::anyhow!(
            "{}",
            capability
                .reason
                .unwrap_or_else(|| "Notifications are unavailable.".to_string())
        ));
    }

    #[cfg(target_os = "macos")]
    {
        deliver::deliver(request)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Ok(())
    }
}

/// Withdraw a scheduled notification that macOS has not delivered yet (specs/0075 W2): a
/// recording started, or the in-app start alert fired first. A no-op for an unknown id or one
/// already delivered. Gated like every other entry point.
pub fn cancel_pending(id: &str) -> Result<()> {
    let capability = capability();
    if !capability.supported {
        return Err(anyhow::anyhow!(
            "{}",
            capability
                .reason
                .unwrap_or_else(|| "Notifications are unavailable.".to_string())
        ));
    }

    #[cfg(target_os = "macos")]
    {
        deliver::cancel_pending(id);
        log::info!("notifications: cancelled pending {id}");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        Ok(())
    }
}

/// Take a delivered (or still pending) notification down now (specs/0074 W5): acting on the in-app twin of a
/// banner removes the banner, so the same question is not left asked twice. Gated like
/// every other entry point — outside an `.app` the framework aborts the process.
pub fn remove(id: &str) -> Result<()> {
    let capability = capability();
    if !capability.supported {
        return Err(anyhow::anyhow!(
            "{}",
            capability
                .reason
                .unwrap_or_else(|| "Notifications are unavailable.".to_string())
        ));
    }

    #[cfg(target_os = "macos")]
    {
        deliver::remove(id);
        log::info!("notifications: removed {id}");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        Ok(())
    }
}

/// Install the delegate and register the categories. Called once from setup; a no-op when
/// the build cannot deliver, so the dev binary launches normally.
pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<()> {
    if !capability().supported {
        log::info!("notifications: not installing (unbundled build)");
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        delegate::install(app.clone())?;
        deliver::register_categories();
        log::info!("notifications: UNUserNotificationCenter delegate installed");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate is the crash-prevention, and this test binary is exactly the environment it
    /// guards: `cargo test` runs unbundled, so `capability()` must report unsupported and
    /// every entry point must refuse *without* reaching Objective-C. If the gate is ever
    /// dropped, this does not fail politely — the process aborts on an uncaught
    /// `NSInternalInconsistencyException`, which is the failure it is protecting users from.
    #[tokio::test]
    async fn bundle_gate_holds() {
        let capability = capability();
        assert!(
            !capability.supported,
            "the test binary is unbundled; if this reports supported, the gate is reading \
             something other than the bundle identifier"
        );
        assert!(capability.reason.is_some(), "an unsupported build must say why");

        assert_eq!(authorization_status().await, "unavailable");
        assert!(request_authorization().await.is_err());
        assert!(deliver(DeliverRequest {
            id: "t".into(),
            title: "t".into(),
            body: "b".into(),
            category: Some(CATEGORY_PLAIN.into()),
            user_info: HashMap::new(),
            auto_dismiss_ms: None,
            deliver_at_ms: None,
        })
        .is_err());
        // The auto-dismiss path (2026-09-21) reaches the framework too, on a delay. Its
        // task re-checks the gate before touching `UNUserNotificationCenter`, and a request
        // that never delivered never schedules one — so a refused deliver must leave
        // nothing behind that could abort the process 8 seconds later.
        assert!(deliver(DeliverRequest {
            id: "t-dismiss".into(),
            title: "t".into(),
            body: "b".into(),
            category: Some(CATEGORY_PREP.into()),
            user_info: HashMap::new(),
            auto_dismiss_ms: Some(1),
            deliver_at_ms: None,
        })
        .is_err());
        // specs/0075 W2: a scheduled request is refused before the trigger is built.
        assert!(deliver(DeliverRequest {
            id: "t-scheduled".into(),
            title: "t".into(),
            body: "b".into(),
            category: Some(CATEGORY_MEETING.into()),
            user_info: HashMap::new(),
            auto_dismiss_ms: None,
            deliver_at_ms: Some(i64::MAX),
        })
        .is_err());
        assert!(cancel_pending("t-scheduled").is_err());
        // specs/0074 W5: the cross-dismiss removal is an entry point too.
        assert!(remove("t").is_err());
    }

    // Owner feedback 2026-09-21: "the 'meeting starts now - join & record' should be
    // persistent, but all others transient." macOS has no per-notification style, so this
    // policy — plus the programmatic removal it drives — is the whole implementation.
    #[test]
    fn only_the_meeting_is_starting_alerts_persist() {
        assert_eq!(
            default_auto_dismiss_ms(Some(CATEGORY_MEETING)),
            None,
            "\"starting now — Join & Record\" must survive until dismissed"
        );
        assert_eq!(
            default_auto_dismiss_ms(Some(CATEGORY_RECORD)),
            None,
            "the link-less twin of the same moment, same stakes"
        );
    }

    #[test]
    fn every_other_category_goes_on_its_own() {
        assert_eq!(default_auto_dismiss_ms(Some(CATEGORY_PREP)), Some(TRANSIENT_MS));
        assert_eq!(default_auto_dismiss_ms(Some(CATEGORY_PLAIN)), Some(TRANSIENT_MS));
        // An unregistered category still delivers (macOS drops the buttons, not the
        // banner), so it needs a policy rather than falling through to "persist forever".
        assert_eq!(default_auto_dismiss_ms(Some("nixon.something.new")), Some(TRANSIENT_MS));
        assert_eq!(default_auto_dismiss_ms(None), Some(TRANSIENT_MS));
    }

    #[test]
    fn transient_is_long_enough_to_read_and_short_enough_not_to_stack() {
        assert!((4_000..=15_000).contains(&TRANSIENT_MS));
    }

    #[test]
    fn auto_dismiss_ms_is_an_optional_override_on_the_wire() {
        let default: DeliverRequest =
            serde_json::from_str(r#"{"id":"a1","title":"T","body":"B"}"#).unwrap();
        assert_eq!(default.auto_dismiss_ms, None, "absent => the category decides");

        let explicit: DeliverRequest = serde_json::from_str(
            r#"{"id":"a1","title":"T","body":"B","autoDismissMs":2500}"#,
        )
        .unwrap();
        assert_eq!(explicit.auto_dismiss_ms, Some(2500));
    }

    #[test]
    fn deliver_request_accepts_the_minimum_the_frontend_sends() {
        let parsed: DeliverRequest =
            serde_json::from_str(r#"{"id":"a1","title":"T","body":"B"}"#).unwrap();
        assert_eq!(parsed.category, None);
        assert!(parsed.user_info.is_empty());
    }

    #[test]
    fn deliver_request_carries_user_info_back_out() {
        let parsed: DeliverRequest = serde_json::from_str(
            r#"{"id":"a1","title":"T","body":"B","category":"nixon.meeting",
                "userInfo":{"meetingId":"m1"}}"#,
        )
        .unwrap();
        assert_eq!(parsed.category.as_deref(), Some(CATEGORY_MEETING));
        assert_eq!(parsed.user_info.get("meetingId").unwrap(), "m1");
    }

    // specs/0075 W2 — the T-0 banner is scheduled with macOS at T-5.
    #[test]
    fn a_future_instant_is_scheduled_for_the_remaining_seconds() {
        let now = 1_800_000_000_000;
        assert_eq!(schedule_delay_secs(Some(now + 300_000), now), Some(300.0));
        assert_eq!(schedule_delay_secs(Some(now + 1_500), now), Some(1.5));
    }

    #[test]
    fn absent_past_or_imminent_instants_deliver_now() {
        let now = 1_800_000_000_000;
        assert_eq!(schedule_delay_secs(None, now), None);
        assert_eq!(schedule_delay_secs(Some(now - 60_000), now), None, "already past");
        assert_eq!(schedule_delay_secs(Some(now), now), None);
        assert_eq!(
            schedule_delay_secs(Some(now + SCHEDULE_MIN_LEAD_MS), now),
            None,
            "one second or less is not worth a trigger"
        );
        assert_eq!(schedule_delay_secs(Some(i64::MIN), now), None, "overflow is not a panic");
    }

    fn request(category: &str, auto_dismiss_ms: Option<u64>) -> DeliverRequest {
        DeliverRequest {
            id: "r".into(),
            title: "t".into(),
            body: "b".into(),
            category: Some(category.into()),
            user_info: HashMap::new(),
            auto_dismiss_ms,
            deliver_at_ms: None,
        }
    }

    #[test]
    fn a_scheduled_request_never_starts_the_auto_dismiss_timer() {
        // The timer would count from scheduling, not delivery, and remove the pending banner.
        assert_eq!(effective_auto_dismiss_ms(&request(CATEGORY_PREP, None), true), None);
        assert_eq!(effective_auto_dismiss_ms(&request(CATEGORY_PLAIN, Some(2_500)), true), None);
        assert_eq!(effective_auto_dismiss_ms(&request(CATEGORY_MEETING, None), true), None);
    }

    #[test]
    fn an_immediate_request_keeps_the_auto_dismiss_policy() {
        let prep = request(CATEGORY_PREP, None);
        assert_eq!(effective_auto_dismiss_ms(&prep, false), Some(TRANSIENT_MS));
        let explicit = request(CATEGORY_PLAIN, Some(2_500));
        assert_eq!(effective_auto_dismiss_ms(&explicit, false), Some(2_500));
        // The start categories persist whether delivered now or scheduled.
        assert_eq!(effective_auto_dismiss_ms(&request(CATEGORY_MEETING, None), false), None);
        assert_eq!(effective_auto_dismiss_ms(&request(CATEGORY_RECORD, None), false), None);
    }

    #[test]
    fn deliver_at_ms_is_optional_camel_case_on_the_wire() {
        let now: DeliverRequest =
            serde_json::from_str(r#"{"id":"a1","title":"T","body":"B"}"#).unwrap();
        assert_eq!(now.deliver_at_ms, None, "absent => deliver now, as before");

        let scheduled: DeliverRequest = serde_json::from_str(
            r#"{"id":"meeting-start-e1","title":"T","body":"B","deliverAtMs":1800000300000}"#,
        )
        .unwrap();
        assert_eq!(scheduled.deliver_at_ms, Some(1_800_000_300_000));
    }
}
