// Permission, asked of macOS rather than assumed (specs/0068).
//
// Both calls bridge a UserNotifications completion block to async Rust over a oneshot,
// exactly as `calendar/eventkit.rs` does for EventKit: the framework objects are `!Send`,
// so all FFI is confined to a synchronous block and only the receiver crosses the await.
// The centre and the block are leaked deliberately — the OS invokes the handler later, on
// an arbitrary queue, and a dropped block there is a use-after-free.
//
// Callers must have checked `super::capability()` first; these functions touch the
// framework immediately.

use std::cell::Cell;

use anyhow::{anyhow, Result};
use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::NSError;
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
};

/// What Nixon asks for: a banner, and a sound with it. No badge — Nixon has nothing to
/// count — and no provisional/critical escalation.
fn requested_options() -> UNAuthorizationOptions {
    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound
}

/// Read the authorization status without prompting.
pub async fn status() -> String {
    let (tx, rx) = tokio::sync::oneshot::channel::<UNAuthorizationStatus>();

    {
        let tx = Cell::new(Some(tx));
        let center = UNUserNotificationCenter::currentNotificationCenter();

        let completion = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the framework hands us a live, non-null settings object for the
            // duration of the block.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            if let Some(sender) = tx.take() {
                let _ = sender.send(status);
            }
        });

        center.getNotificationSettingsWithCompletionHandler(&completion);

        std::mem::forget(center);
        std::mem::forget(completion);
    }

    match rx.await {
        Ok(status) => describe(status).to_string(),
        Err(_) => {
            log::warn!("notifications: settings query did not complete");
            "unavailable".to_string()
        }
    }
}

/// Request permission. The macOS dialog appears on the first call of the app's life; every
/// later call returns the stored answer silently, which is why the UI has to offer a route
/// into System Settings once the answer is "denied".
pub async fn request() -> Result<bool> {
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();

    {
        let tx = Cell::new(Some(tx));
        let center = UNUserNotificationCenter::currentNotificationCenter();

        let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            log::info!(
                "notifications: authorization completion fired (granted={})",
                granted.as_bool()
            );
            if let Some(sender) = tx.take() {
                let _ = sender.send(granted.as_bool());
            }
        });

        center.requestAuthorizationWithOptions_completionHandler(requested_options(), &completion);

        std::mem::forget(center);
        std::mem::forget(completion);
    }

    rx.await
        .map_err(|_| anyhow!("The notification permission request did not complete."))
}

/// Apple's status constants, in the vocabulary the frontend uses.
fn describe(status: UNAuthorizationStatus) -> &'static str {
    if status == UNAuthorizationStatus::NotDetermined {
        "not_determined"
    } else if status == UNAuthorizationStatus::Denied {
        "denied"
    } else if status == UNAuthorizationStatus::Authorized {
        "authorized"
    } else if status == UNAuthorizationStatus::Provisional {
        "provisional"
    } else {
        // Ephemeral (App Clips) and anything Apple adds later: not something Nixon can act
        // on, and not a denial we should report as one.
        "unavailable"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_constant_has_a_name() {
        assert_eq!(describe(UNAuthorizationStatus::NotDetermined), "not_determined");
        assert_eq!(describe(UNAuthorizationStatus::Denied), "denied");
        assert_eq!(describe(UNAuthorizationStatus::Authorized), "authorized");
        assert_eq!(describe(UNAuthorizationStatus::Provisional), "provisional");
        assert_eq!(describe(UNAuthorizationStatus::Ephemeral), "unavailable");
    }

    #[test]
    fn asks_for_a_banner_and_a_sound_only() {
        let options = requested_options();
        assert!(options.contains(UNAuthorizationOptions::Alert));
        assert!(options.contains(UNAuthorizationOptions::Sound));
        assert!(!options.contains(UNAuthorizationOptions::Badge));
    }
}
