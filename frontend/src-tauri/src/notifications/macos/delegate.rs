// The delegate — the only path a button press can take back into Nixon (specs/0068).
//
// `UNUserNotificationCenterDelegate` is what makes an OS notification *actionable*: macOS
// calls it when the banner is pressed, whether or not Nixon has a window open, which is the
// entire point of the feature. It is also the one genuinely unsafe piece in this module, so
// two invariants are load-bearing:
//
//   1. `setDelegate:` is a WEAK property. A delegate that Rust drops leaves macOS holding a
//      dangling pointer, and the crash lands later, on the press. Ours is leaked on purpose
//      and installed exactly once.
//   2. Every selector must call its completion handler exactly once, on every path. Miss it
//      and macOS logs a warning and eventually stops delivering to the app.
//
// The handle the event is emitted on lives in a `OnceLock` closure rather than in class
// ivars, so the class stays generic-free while `install` keeps working for any
// `tauri::Runtime`.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread};
use objc2_user_notifications::{
    UNNotification, UNNotificationPresentationOptions, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier,
};
use tauri::{AppHandle, Emitter, Runtime};

use super::{ActionPayload, ACTION_OPEN, EVENT_ACTION};

/// Where a press goes. Set once by [`install`]; the delegate holds no state of its own.
static SINK: OnceLock<Box<dyn Fn(ActionPayload) + Send + Sync>> = OnceLock::new();

define_class!(
    // SAFETY:
    // - NSObject imposes no subclassing requirements.
    // - The type implements no Drop; it is leaked into `setDelegate:` for the process
    //   lifetime, which is what that weak property requires.
    #[unsafe(super(NSObject))]
    #[name = "NixonNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Asked when a notification arrives while Nixon is the front app. The default is
        /// to suppress it, which is exactly the case someone testing the feature tries
        /// first — so answer with the banner and the Notification Center entry.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((
                UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,
            ));
        }

        /// A button, or the banner body, was pressed.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier().to_string();
            let id = response.notification().request().identifier().to_string();
            dispatch(&action, &id);
            // Unconditional, and last: macOS wants this exactly once however we handled it.
            completion.call(());
        }
    }
);

/// Route one press, translating Apple's identifiers into Nixon's.
///
/// Split out of the selector so it can be tested without an Objective-C response object —
/// the mapping (what counts as a tap, what is dropped) is the part with decisions in it.
fn dispatch(action_identifier: &str, notification_id: &str) {
    let Some(action) = normalize(action_identifier) else {
        // Dismissed — swiped away, or "Clear". Not an instruction to do anything.
        log::debug!("notifications: {notification_id} dismissed");
        return;
    };

    let payload = super::deliver::payload_for(&action, notification_id);
    log::info!("notifications: {notification_id} → {action}");
    match SINK.get() {
        Some(sink) => sink(payload),
        // Pressed before `install` ran, which should be impossible: the delegate is set by
        // the same call that fills the sink.
        None => log::warn!("notifications: press arrived with no sink installed"),
    }
}

/// Apple's identifiers, in Nixon's vocabulary. `None` means "do nothing".
fn normalize(action_identifier: &str) -> Option<String> {
    let default_tap = unsafe { UNNotificationDefaultActionIdentifier.to_string() };
    let dismiss = unsafe { UNNotificationDismissActionIdentifier.to_string() };

    if action_identifier == dismiss {
        None
    } else if action_identifier == default_tap {
        Some(ACTION_OPEN.to_string())
    } else {
        Some(action_identifier.to_string())
    }
}

/// Install the delegate and point presses at `app`. Safe to call more than once; only the
/// first call takes effect.
pub fn install<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    if SINK
        .set(Box::new(move |payload: ActionPayload| {
            if let Err(error) = app.emit(EVENT_ACTION, payload) {
                log::warn!("notifications: could not emit {EVENT_ACTION}: {error}");
            }
        }))
        .is_err()
    {
        log::debug!("notifications: delegate already installed");
        return Ok(());
    }

    let delegate: Retained<Delegate> = {
        let this = Delegate::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    };

    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // `delegate` is held weakly by the centre: it must outlive this scope or the first
    // press dereferences freed memory. `center` is leaked for the same reason the EventKit
    // store is — the framework calls back into it long after we return.
    std::mem::forget(delegate);
    std::mem::forget(center);

    if SINK.get().is_none() {
        return Err(anyhow!("notification sink was not installed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dismissal_is_not_an_instruction() {
        let dismiss = unsafe { UNNotificationDismissActionIdentifier.to_string() };
        assert_eq!(normalize(&dismiss), None);
    }

    #[test]
    fn tapping_the_banner_reads_as_open() {
        let tap = unsafe { UNNotificationDefaultActionIdentifier.to_string() };
        assert_eq!(normalize(&tap).as_deref(), Some(ACTION_OPEN));
    }

    #[test]
    fn a_button_keeps_the_identifier_it_was_registered_with() {
        assert_eq!(
            normalize(super::super::ACTION_JOIN_AND_RECORD).as_deref(),
            Some(super::super::ACTION_JOIN_AND_RECORD)
        );
    }
}
