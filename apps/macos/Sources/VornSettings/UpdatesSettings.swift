import SwiftUI
import VornCore
import VornUI

extension UpdateStatusView.Tone {
    var dot: Color {
        switch self {
        case .blocked: Theme.bronzo
        case .broken: Theme.danger
        case .live: Theme.ink
        case .settled: Theme.inkFaint
        case .idle: Theme.inkGhost
        }
    }
}

/// UpdatesSettings.tsx.
struct UpdatesSettings: View {
    @Bindable var model: SettingsModel

    var body: some View {
        if model.config != nil {
            let d = model.defaults
            let channel = d["updateChannel"]?.stringValue ?? "stable"
            let status = model.updater?.status ?? model.updateStatus
            let view = UpdateStatusView(status, channel: channel)
            let cost = view.action == .restart
                ? updateCostLine(sessionCount: model.restartFacing.count, aTurnIsRunning: model.restartFacing.running, sessionsSurvive: true)
                : nil
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "Updates", description: "How Vorn keeps itself current")
                HStack(spacing: 12) {
                    Circle().fill(view.tone.dot).frame(width: 5, height: 5)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(view.label).font(.ui(13)).foregroundStyle(Theme.gray200)
                        if let cost {
                            Text(cost).font(.ui(12)).foregroundStyle(Theme.bronzo)
                        } else if let detail = view.detail {
                            Text(detail).font(.ui(12)).foregroundStyle(Theme.gray500)
                        }
                        if let p = view.percent {
                            GeometryReader { g in
                                ZStack(alignment: .leading) {
                                    Capsule().fill(Theme.white(0.08))
                                    Capsule().fill(Theme.white(0.3)).frame(width: g.size.width * CGFloat(p) / 100)
                                }
                            }
                            .frame(height: 3)
                            .padding(.top, 6)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if let action = view.action {
                        let lead = action == .restart
                        QuietButton(label: action == .restart ? "Restart Now" : action == .download ? "Download" : "Retry",
                                    fontSize: 12, fill: Theme.white(lead ? 0.1 : 0.06), hoverFill: Theme.white(lead ? 0.14 : 0.1),
                                    color: lead ? .white : Theme.gray300, padding: (12, 4)) {
                            switch action {
                            case .restart: model.updater?.install()
                            case .download: model.updater?.download()
                            case .retry: model.updater?.check()
                            }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.03)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
                .padding(.bottom, 20)

                VStack(spacing: 4) {
                    SettingRow(label: "Current version", description: "Vorn \(model.appVersion)") {
                        QuietButton(label: status.isChecking ? "Checking…" : "Check Now",
                                    disabled: status.isChecking || status.isUnsupported) { model.updater?.check() }
                    }
                    SettingRow(label: "Update channel", description: "Beta receives early releases; stable receives tested releases only") {
                        SegmentedControl(options: [SegmentOption(value: "stable", label: "Stable"), SegmentOption(value: "beta", label: "Beta")],
                                         value: channel) { ch in
                            model.updateDefaults(["updateChannel": .string(ch)])
                            model.updater?.setChannel(ch)
                        }
                    }
                    SettingRow(label: "Download automatically", description: "Installing still waits for you to restart") {
                        ToggleSwitch(checked: d["updateAutoDownload"]?.boolValue != false) { on in
                            model.updateDefaults(["updateAutoDownload": .bool(on)])
                            model.updater?.setAutoDownload(on)
                        }
                    }
                }
            }
        }
    }
}

/// SidebarFooter.tsx's update notice: the pending version, what restarting costs, and the button.
public struct UpdateBanner: View {
    let status: UpdateStatus
    let cost: String?
    let onDismiss: () -> Void
    let onRestart: () -> Void

    public init(status: UpdateStatus, cost: String?, onDismiss: @escaping () -> Void, onRestart: @escaping () -> Void) {
        self.status = status
        self.cost = cost
        self.onDismiss = onDismiss
        self.onRestart = onRestart
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Circle().fill(Theme.bronzo).frame(width: 5, height: 5)
                Text(UpdateStatusView(status).shortLabel).font(.ui(11.5)).foregroundStyle(Theme.gray200).lineLimit(1)
                Spacer(minLength: 0)
                Hovering { hover in
                    Button(action: onDismiss) {
                        Text("✕").font(.ui(11)).foregroundStyle(hover ? Theme.gray300 : Theme.gray600)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Dismiss update notice")
                }
            }
            if let cost {
                Text(cost).font(.ui(10.5)).foregroundStyle(Theme.bronzo).padding(.top, 6).fixedSize(horizontal: false, vertical: true)
            }
            Hovering { hover in
                Button(action: onRestart) {
                    Text("Restart to update").font(.ui(11)).foregroundStyle(Theme.gray300)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(hover ? 0.08 : 0.04)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.top, 8)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.03)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .padding(.horizontal, 8)
        .padding(.bottom, 6)
    }
}
