import UserNotifications

/// The OS side of the notifications page: asking once.
public enum NotificationPermission {
    /// The notification center only exists for a bundled app; a bare build has none.
    public static var available: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    public static func request() {
        guard available else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }
}
