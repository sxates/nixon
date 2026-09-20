// Building and posting the banner (specs/0068).
//
// Categories are the reason this module exists in the shape it does: a macOS notification
// gets its button by naming a category that was registered up front, so the actions are
// declared once at setup and every later alert just carries the category id. Each category
// here carries at most ONE action — see `CATEGORY_MEETING` for the measurement behind that.
// An unregistered id is not an error — macOS delivers the banner without buttons — which is
// why `CATEGORY_PLAIN` can be an empty category rather than a special case.
//
// Callers must have checked `super::capability()` first.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use objc2_foundation::{NSArray, NSSet, NSString};
use objc2_user_notifications::{
    UNMutableNotificationContent, UNNotificationAction, UNNotificationActionOptions,
    UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationRequest,
    UNNotificationSound, UNUserNotificationCenter,
};

use super::{
    ActionPayload, DeliverRequest, ACTION_JOIN_AND_RECORD, ACTION_PREP, ACTION_RECORD,
    CATEGORY_MEETING, CATEGORY_PLAIN, CATEGORY_PREP, CATEGORY_RECORD,
};

/// `id` → the `user_info` its sender attached.
///
/// Kept here rather than in the notification's `userInfo` so the Objective-C content stays
/// title/body/category and nothing has to be marshalled back out of a dictionary. A banner
/// only matters while Nixon is running, so an in-memory map is the right lifetime.
type Remembered = Vec<(String, HashMap<String, String>)>;

static USER_INFO: Mutex<Option<Remembered>> = Mutex::new(None);

/// How many deliveries to remember. Alerts are one per meeting per session; this is a cap
/// against a pathological loop, not a working set.
const REMEMBERED: usize = 64;

/// Register the categories. Idempotent — macOS replaces the whole set each call.
pub fn register_categories() {
    // Foreground: it is about to start recording, and a recording that begins with no
    // visible sign of it is worse than one extra window coming forward.
    let join_and_record = action(
        ACTION_JOIN_AND_RECORD,
        "Join & Record",
        UNNotificationActionOptions::Foreground,
    );

    let meeting = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY_MEETING),
        &NSArray::from_retained_slice(&[join_and_record]),
        &NSArray::from_slice(&[]),
        UNNotificationCategoryOptions::empty(),
    );

    let prep = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY_PREP),
        &NSArray::from_retained_slice(&[action(
            ACTION_PREP,
            "Prep",
            UNNotificationActionOptions::Foreground,
        )]),
        &NSArray::from_slice(&[]),
        UNNotificationCategoryOptions::empty(),
    );

    let record = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY_RECORD),
        &NSArray::from_retained_slice(&[action(
            ACTION_RECORD,
            "Record",
            UNNotificationActionOptions::Foreground,
        )]),
        &NSArray::from_slice(&[]),
        UNNotificationCategoryOptions::empty(),
    );

    let plain = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY_PLAIN),
        &NSArray::from_slice(&[]),
        &NSArray::from_slice(&[]),
        UNNotificationCategoryOptions::empty(),
    );

    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setNotificationCategories(&NSSet::from_retained_slice(&[meeting, prep, record, plain]));
}

fn action(
    id: &str,
    title: &str,
    options: UNNotificationActionOptions,
) -> objc2::rc::Retained<UNNotificationAction> {
    UNNotificationAction::actionWithIdentifier_title_options(
        &NSString::from_str(id),
        &NSString::from_str(title),
        options,
    )
}

/// Post a notification. Returns as soon as it is handed to the OS; delivery failures arrive
/// on the completion handler, which we only log — there is nothing the caller could do with
/// them that it would not already do with a banner the user ignored.
pub fn deliver(request: DeliverRequest) -> Result<()> {
    remember(&request.id, request.user_info.clone());

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&request.title));
    content.setBody(&NSString::from_str(&request.body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    if let Some(category) = request.category.as_deref() {
        content.setCategoryIdentifier(&NSString::from_str(category));
    }

    // A nil trigger means "deliver now" — Nixon decides when to alert (it already polls on
    // a 60s tick), so nothing here is scheduled. See the spec's non-goals.
    let un_request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&request.id),
        &content,
        None,
    );

    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.addNotificationRequest_withCompletionHandler(&un_request, None);
    log::info!(
        "notifications: delivered {} (category={:?})",
        request.id,
        request.category
    );
    Ok(())
}

fn remember(id: &str, user_info: HashMap<String, String>) {
    let mut guard = match USER_INFO.lock() {
        Ok(guard) => guard,
        // A poisoned lock here would mean a panic inside the delegate; losing the routing
        // data is better than taking the app down with it.
        Err(poisoned) => poisoned.into_inner(),
    };
    let entries: &mut Remembered = guard.get_or_insert_with(Vec::new);
    entries.retain(|(known, _)| known != id);
    entries.push((id.to_string(), user_info));
    let overflow = entries.len().saturating_sub(REMEMBERED);
    entries.drain(..overflow);
}

/// Build the payload for a press: the action, the notification it came from, and whatever
/// the sender attached. An id we do not know still produces a payload — the frontend drops
/// it — because a banner can outlive the state that created it.
pub fn payload_for(action_id: &str, notification_id: &str) -> ActionPayload {
    let user_info = match USER_INFO.lock() {
        Ok(guard) => guard
            .as_ref()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|(id, _)| id == notification_id)
                    .map(|(_, info)| info.clone())
            })
            .unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    ActionPayload {
        action_id: action_id.to_string(),
        notification_id: notification_id.to_string(),
        user_info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_carries_back_what_the_sender_attached() {
        remember("m-1", HashMap::from([("meetingId".into(), "abc".into())]));
        let payload = payload_for(ACTION_JOIN_AND_RECORD, "m-1");
        assert_eq!(payload.action_id, ACTION_JOIN_AND_RECORD);
        assert_eq!(payload.user_info.get("meetingId").unwrap(), "abc");
    }

    #[test]
    fn an_unknown_banner_still_produces_a_payload() {
        let payload = payload_for(ACTION_JOIN_AND_RECORD, "never-sent");
        assert!(payload.user_info.is_empty());
        assert_eq!(payload.notification_id, "never-sent");
    }

    #[test]
    fn re_delivering_an_id_replaces_its_user_info_rather_than_stacking_it() {
        remember("m-2", HashMap::from([("v".into(), "first".into())]));
        remember("m-2", HashMap::from([("v".into(), "second".into())]));
        assert_eq!(payload_for(ACTION_JOIN_AND_RECORD, "m-2").user_info.get("v").unwrap(), "second");
    }

    #[test]
    fn the_map_does_not_grow_without_bound() {
        for i in 0..(REMEMBERED + 20) {
            remember(&format!("bulk-{i}"), HashMap::new());
        }
        let guard = USER_INFO.lock().unwrap();
        assert!(guard.as_ref().unwrap().len() <= REMEMBERED);
    }
}
