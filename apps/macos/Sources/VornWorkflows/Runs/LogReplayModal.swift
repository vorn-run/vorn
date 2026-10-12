import AppKit
import SwiftUI
import VornCore
import VornUI

/// LogReplayModal.tsx: a step's whole output, scrolled to its end, with Copy; Escape or the X closes it.
public struct LogReplayModal: View {
    let logs: String
    let onClose: () -> Void
    @State private var copied = false

    public init(logs: String, onClose: @escaping () -> Void) {
        self.logs = logs
        self.onClose = onClose
    }

    public var body: some View {
        GeometryReader { geo in
            ZStack {
                Color.black.opacity(0.6).ignoresSafeArea()
                VStack(spacing: 0) {
                    header
                    ScrollViewReader { proxy in
                        ScrollView([.vertical]) {
                            Text(logs)
                                .font(.system(size: 12, design: .monospaced))
                                .lineSpacing(5)
                                .foregroundStyle(Theme.gray300)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(16)
                                .id("end")
                        }
                        .onAppear { proxy.scrollTo("end", anchor: .bottom) }
                    }
                }
                .frame(width: min(geo.size.width * 0.9, 900), height: geo.size.height * 0.8)
                .background(Theme.surfaceBase, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.1), lineWidth: 1))
                .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
                .shadow(color: .black.opacity(0.5), radius: 25, y: 12)
            }
        }
        .background {
            Button("", action: onClose).keyboardShortcut(.cancelAction).opacity(0)
        }
    }

    private var header: some View {
        HStack {
            HStack(spacing: 12) {
                Text("Full Output").font(.system(size: 13, weight: .medium)).foregroundStyle(Color.white)
                Text("\(logs.components(separatedBy: "\n").count) lines").font(.system(size: 11)).foregroundStyle(Theme.gray600)
            }
            Spacer()
            HStack(spacing: 8) {
                Hovering { hovered in
                    Button(action: copy) {
                        HStack(spacing: 6) {
                            LucideIcon(copied ? .circleCheckBig : .copy, size: 12)
                            Text(copied ? "Copied" : "Copy")
                        }
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(hovered ? Theme.gray200 : Theme.gray400)
                        .padding(.horizontal, 10).padding(.vertical, 4)
                        .background(Theme.white(hovered ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.06), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                }
                Hovering { hovered in
                    Button(action: onClose) {
                        LucideIcon(.x, size: 14).foregroundStyle(hovered ? .white : Theme.gray500).padding(4)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(logs, forType: .string)
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }
}
