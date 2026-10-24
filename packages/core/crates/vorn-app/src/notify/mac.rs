//! The macOS notification center, used when Vorn runs as Vorn.app: notices
//! carry Vorn's name and icon, the first one asks macOS for permission, and
//! a refusal is remembered so Settings can say so.
//!
//! Outside a bundle the center aborts the process, so [`available`] gates
//! every call and the caller falls back to `osascript`.

use std::sync::atomic::Ordering;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
};

use super::BLOCKED;

/// Whether this process is an app bundle the notification center accepts.
pub fn available() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// The bundle's id, for System Settings' link to Vorn's notifications.
pub fn bundle_id() -> Option<String> {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}

/// Asks macOS whether Vorn may notify, remembering a refusal.
pub fn refresh() {
    if !available() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let answer = RcBlock::new(|settings: std::ptr::NonNull<UNNotificationSettings>| {
        // SAFETY: the center hands a valid settings object for the block's duration.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        BLOCKED.store(status == UNAuthorizationStatus::Denied, Ordering::Relaxed);
    });
    center.getNotificationSettingsWithCompletionHandler(&answer);
}

/// Shows a notice, asking permission first; macOS asks the person only once.
pub fn show(title: String, body: String, sound: Option<String>) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let granted = RcBlock::new(move |ok: Bool, _: *mut NSError| {
        BLOCKED.store(!ok.as_bool(), Ordering::Relaxed);
        if ok.as_bool() {
            post(&title, &body, sound.as_deref());
        }
    });
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
    center.requestAuthorizationWithOptions_completionHandler(options, &granted);
}

fn post(title: &str, body: &str, sound: Option<&str>) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let sound = sound.map(|name| match name {
        "default" => UNNotificationSound::defaultSound(),
        name => UNNotificationSound::soundNamed(&NSString::from_str(name)),
    });
    content.setSound(sound.as_deref());
    let id = NSString::from_str(&format!(
        "vorn-{}-{}",
        std::process::id(),
        super::os_shown()
    ));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
}
