import AppKit
import SwiftUI
import VornUI

/// What the window shows when it cannot carry on (ErrorBoundary.tsx): the error,
/// a way to load again, and a way to copy it for a report.
struct ErrorScreen: View {
    let error: String
    let reload: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Something went wrong")
                .font(.ui(18, weight: .semibold))
                .foregroundStyle(Color(red: 0.973, green: 0.443, blue: 0.443))
                .padding(.bottom, 8)
            Text("The app encountered an unexpected error. You can try reloading or check the details below.")
                .font(.ui(13))
                .foregroundStyle(Theme.gray400)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 16)
            ScrollView {
                Text(error)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Theme.gray300)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
            .frame(maxHeight: 300)
            .fixedSize(horizontal: false, vertical: true)
            .background(Theme.surfaceSunken, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.hairline, lineWidth: 1))
            .padding(.bottom, 16)
            HStack(spacing: 8) {
                ErrorButton(label: "Reload", fill: Theme.white(0.08), border: Theme.white(0.1), color: Theme.gray200,
                            action: reload)
                ErrorButton(label: "Copy error", fill: .clear, border: Theme.hairline, color: Theme.gray400) {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(error, forType: .string)
                }
            }
        }
        .frame(maxWidth: 560)
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.surfaceOverlay)
    }
}

private struct ErrorButton: View {
    let label: String
    let fill: Color
    let border: Color
    let color: Color
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.ui(13))
                .foregroundStyle(color)
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .background(fill, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(border, lineWidth: 1))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
