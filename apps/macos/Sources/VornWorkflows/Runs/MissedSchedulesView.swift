import SwiftUI
import VornCore
import VornUI

/// MissedScheduleDialog.tsx: schedules that came due while Vorn was closed, to run now or skip.
struct MissedSchedulesView: View {
    let store: WorkflowsStore

    var body: some View {
        let surfaces = store.surfaces
        RunModal(backdrop: 0.5) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        LucideIcon(.triangleAlert, size: 16).foregroundStyle(Theme.amber400)
                        Text("Missed Schedules").font(.system(size: 16, weight: .medium)).foregroundStyle(Color.white)
                    }
                    Text("These workflows were scheduled while Vorn was closed.")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.gray500)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
                Hairline(opacity: 0.06)
                VStack(spacing: 8) {
                    ForEach(surfaces.missed) { item in
                        row(item, checked: surfaces.missedSelected.contains(item.workflowId))
                    }
                }
                .padding(16)
                Hairline(opacity: 0.06)
                HStack(spacing: 12) {
                    Spacer(minLength: 0)
                    Hovering { hovered in
                        Button(action: store.skipMissed) {
                            Text("Skip All")
                                .font(.system(size: 14))
                                .foregroundStyle(hovered ? Theme.gray200 : Theme.gray400)
                                .padding(.horizontal, 16)
                                .padding(.vertical, 8)
                                .background(Theme.white(hovered ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    let count = surfaces.missedSelected.count
                    Hovering { hovered in
                        Button(action: store.runMissedSelected) {
                            Text("Run Selected (\(count))")
                                .font(.system(size: 14, weight: .medium))
                                .foregroundStyle(Color.white)
                                .padding(.horizontal, 16)
                                .padding(.vertical, 8)
                                .background(
                                    Theme.white(hovered && count > 0 ? 0.15 : 0.1),
                                    in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(count == 0)
                        .opacity(count == 0 ? 0.3 : 1)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
            }
            .frame(width: 420)
            .modalCard(Theme.surfaceOverlay)
            .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        }
    }

    private func row(_ item: MissedSchedule, checked: Bool) -> some View {
        Hovering { hovered in
            Button { store.toggleMissed(item.workflowId) } label: {
                HStack(spacing: 12) {
                    CheckSquare(checked: checked, tint: Theme.blue500)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item.name).font(.system(size: 14)).foregroundStyle(Theme.gray200)
                        HStack(spacing: 4) {
                            LucideIcon(.clock, size: 10)
                            Text(item.scheduledLabel)
                        }
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.gray500)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(12)
                .background(Theme.white(hovered ? 0.05 : 0.03), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(item.name)
            .accessibilityValue(checked ? "selected" : "not selected")
        }
    }
}
