import AppKit
import UserNotifications
import VornCore
import VornSettings
import VornUI
import VornWorkflows

/// The window's notifications (notifications.ts, run-notifications.ts): a bell or a
/// program's notice from any session, and a workflow's gate, sign-in or end.
@MainActor
final class AppNotifier: NSObject, UNUserNotificationCenterDelegate {
    private weak var store: VornStore?
    private weak var model: AppModel?
    private weak var workflows: WorkflowsStore?
    private var cooldown = NoticeCooldown()
    private var announcer = RunAnnouncer()

    /// Seams for tests: the tone, the OS notification, and whether this window has focus.
    var playTone: (NotificationReason, Double) -> Void = { NotificationTone.play($0, volume: $1) }
    var show: (_ title: String, _ body: String, _ target: AppNotice.Target) async -> Void = AppNotifier.post
    var hasFocus: () -> Bool = { NSApp?.isActive ?? false }
    var permitted: () async -> Bool = AppNotifier.authorized

    func bind(store: VornStore, model: AppModel, workflows: WorkflowsStore) {
        self.store = store
        self.model = model
        self.workflows = workflows
        store.onNotice = { [weak self] notice in self?.terminalNotice(notice) }
        workflows.onRunUpdate = { [weak self] run in self?.runUpdated(run) }
        if NotificationPermission.available { UNUserNotificationCenter.current().delegate = self }
    }

    private var prefs: NotificationPrefs { store?.defaults?.notifications ?? NotificationPrefs() }

    /// Asks the OS once notifications are on, as the window does when its config loads.
    func requestIfEnabled() {
        if prefs.enabled == true { NotificationPermission.request() }
    }

    func terminalNotice(_ notice: AppNotice) {
        guard prefs.wantsBell else { return }
        dispatch(notice)
    }

    func runUpdated(_ run: WorkflowExecution) {
        for announcement in announcer.announce(run, workflow: workflows?.workflow(run.workflowId)) {
            switch announcement {
            case .wait(let notice):
                if prefs.wantsWorkflowWait { dispatch(notice) }
            case .ended(let runId, let body):
                Task {
                    guard await permitted() else { return }
                    await show("Vorn", body, .run(runId))
                }
            }
        }
    }

    /// The tone plays even with the window focused; the OS notice only when it is not.
    private func dispatch(_ notice: AppNotice) {
        let prefs = prefs
        if prefs.soundEnabled == true { playTone(notice.reason, prefs.volume) }
        Task {
            guard await permitted(), !hasFocus(), cooldown.admit(notice.key) else { return }
            await show(notice.title, notice.body, notice.target)
        }
    }

    /// Clicking a notice brings the window forward, then the session or run it is about.
    func open(_ target: AppNotice.Target) {
        NSApp?.activate(ignoringOtherApps: true)
        guard let model else { return }
        switch target {
        case .session(let id):
            model.mainViewMode = .sessions
            model.focus(id)
        case .run(let id):
            model.mainViewMode = .workflows
            workflows?.showRun(id)
        case .none:
            break
        }
    }

    // MARK: OS

    private static func authorized() async -> Bool {
        guard NotificationPermission.available else { return false }
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        return settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional
    }

    private static func post(_ title: String, _ body: String, _ target: AppNotice.Target) async {
        guard NotificationPermission.available else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        switch target {
        case .session(let id): content.userInfo = ["session": id]
        case .run(let id): content.userInfo = ["run": id]
        case .none: break
        }
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification)
        async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse)
        async {
        let info = response.notification.request.content.userInfo
        let target: AppNotice.Target = if let id = info["session"] as? String {
            .session(id)
        } else if let id = info["run"] as? String {
            .run(id)
        } else {
            .none
        }
        await MainActor.run { open(target) }
    }
}
