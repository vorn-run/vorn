import SwiftUI
import VornCore
import VornUI

/// AppearanceSettings.tsx.
struct AppearanceSettings: View {
    @Bindable var model: SettingsModel
    @State private var rowDraft: Double?

    var body: some View {
        if model.config != nil {
            let d = model.defaults
            let font = Int(d["fontSize"]?.numberValue ?? 13)
            let row = rowDraft ?? d["rowHeight"]?.numberValue ?? 208
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "Appearance", description: "Customize how Vorn looks")
                VStack(spacing: 4) {
                    SettingRow(label: "Font Size", description: "Terminal font size in pixels") {
                        HStack(spacing: 8) {
                            stepButton("-") { model.updateDefaults(["fontSize": .number(Double(max(8, font - 1)))]) }
                            Text("\(font)").font(.ui(14)).foregroundStyle(Theme.gray200).frame(width: 32)
                            stepButton("+") { model.updateDefaults(["fontSize": .number(Double(min(24, font + 1)))]) }
                        }
                    }
                    SettingRow(label: "Card Row Height", description: "Terminal card height in pixels (\(Int(row.rounded()))px)") {
                        HStack(spacing: 12) {
                            Slider(value: Binding(get: { row }, set: { rowDraft = $0.rounded() }), in: 100...600) { editing in
                                if !editing, let v = rowDraft {
                                    model.updateDefaults(["rowHeight": .number(v)])
                                    rowDraft = nil
                                }
                            }
                            .controlSize(.small)
                            .tint(Theme.blue500)
                            .frame(width: 128)
                            Text("\(Int(row.rounded()))px").font(.ui(14)).foregroundStyle(Theme.gray400)
                                .frame(width: 48, alignment: .trailing)
                        }
                    }
                    SettingRow(label: "Layout Mode", description: "Choose between grid or tab layout for sessions") {
                        SegmentedControl(options: [
                            SegmentOption(value: "grid", label: "Grid", icon: .layoutGrid),
                            SegmentOption(value: "tabs", label: "Tabs", icon: .layoutTabs),
                        ], value: d["layoutMode"]?.stringValue ?? "grid") { model.updateDefaults(["layoutMode": .string($0)]) }
                    }
                    SettingRow(label: "Default Task View", description: "Choose between list or kanban board for tasks") {
                        SegmentedControl(options: [
                            SegmentOption(value: "list", label: "List", icon: .viewList),
                            SegmentOption(value: "kanban", label: "Kanban", icon: .viewKanban),
                        ], value: d["taskViewMode"]?.stringValue ?? "list") { model.updateDefaults(["taskViewMode": .string($0)]) }
                    }
                }
            }
        }
    }

    private func stepButton(_ label: String, action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                Text(label).font(.ui(14)).foregroundStyle(Theme.gray300)
                    .frame(width: 28, height: 28)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(hover ? 0.1 : 0.06)))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// NotificationSettings.tsx.
struct NotificationSettings: View {
    @Bindable var model: SettingsModel

    private var n: [String: JSONValue] {
        var base: [String: JSONValue] = ["enabled": false, "onWaiting": true, "onError": true, "onBell": true]
        for (k, v) in model.defaults["notifications"]?.objectValue ?? [:] { base[k] = v }
        return base
    }

    private func set(_ patch: [String: JSONValue]) {
        var next = n
        for (k, v) in patch { next[k] = v }
        model.updateDefaults(["notifications": .object(next)])
    }

    var body: some View {
        if model.config != nil {
            let n = n
            let enabled = n["enabled"]?.boolValue == true
            let volume = n["soundVolume"]?.numberValue ?? 0.5
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "Notifications", description: "Get notified when agents need your attention")
                VStack(spacing: 4) {
                    SettingRow(label: "Enable Notifications", description: "Show OS notifications when agents need attention") {
                        ToggleSwitch(checked: enabled) { on in
                            set(["enabled": .bool(on)])
                            if on { NotificationPermission.request() }
                        }
                    }
                    toggle("Waiting for Input", "Notify when an agent is waiting for your response", "onWaiting", enabled)
                    toggle("Errors", "Notify when an agent encounters an error", "onError", enabled)
                    toggle("Terminal Bell", "Notify when an agent explicitly sends a notification signal", "onBell", enabled)
                }
                Text("Sound").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200).padding(.top, 32).padding(.bottom, 8)
                Text("Play audio tones when agents change status").font(.ui(14)).foregroundStyle(Theme.gray500).padding(.bottom, 24)
                VStack(spacing: 12) {
                    SettingRow(label: "Sound Effects", description: "Play a tone on waiting, error, and bell events (works even when focused)",
                               disabled: !enabled) {
                        ToggleSwitch(checked: n["soundEnabled"]?.boolValue ?? false, disabled: !enabled) { set(["soundEnabled": .bool($0)]) }
                    }
                    if n["soundEnabled"]?.boolValue == true && enabled {
                        HStack(spacing: 12) {
                            Text("Volume").font(.ui(12)).foregroundStyle(Theme.gray400).frame(width: 48, alignment: .leading)
                            Slider(value: Binding(get: { volume * 100 }, set: { set(["soundVolume": .number(($0 / 100).rounded(toPlaces: 2))]) }),
                                   in: 0...100).controlSize(.mini).tint(.white)
                            Text("\(Int((volume * 100).rounded()))%").font(.ui(12)).foregroundStyle(Theme.gray500).frame(width: 32, alignment: .trailing)
                            QuietButton(label: "Test", bordered: false, fontSize: 11, radius: Theme.radius, fill: Theme.white(0.06),
                                        hoverFill: Theme.white(0.1), color: Theme.gray400, hoverColor: .white, padding: (8, 4)) {
                                NotificationTone.play(.bell, volume: volume)
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                    }
                }
            }
        }
    }

    private func toggle(_ label: String, _ desc: String, _ key: String, _ enabled: Bool) -> some View {
        SettingRow(label: label, description: desc, disabled: !enabled) {
            ToggleSwitch(checked: n[key]?.boolValue == true, disabled: !enabled) { set([key: .bool($0)]) }
        }
    }
}

extension Double {
    func rounded(toPlaces p: Int) -> Double {
        let m = pow(10, Double(p))
        return (self * m).rounded() / m
    }
}
