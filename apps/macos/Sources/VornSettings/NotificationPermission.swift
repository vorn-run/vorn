import AppKit
import UserNotifications

/// The OS side of the notifications page: asking once, and the test tone.
enum NotificationPermission {
    static func request() {
        guard Bundle.main.bundleIdentifier != nil else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    @MainActor static func bell(volume: Double) {
        guard let sound = NSSound(named: "Glass") else { return NSSound.beep() }
        sound.volume = Float(volume)
        sound.play()
    }
}
