import SwiftUI
import VornCore
import VornUI

/// CommitDialog.tsx: the branch, the totals, a message, and whether to push after committing.
struct CommitDialog: View {
    let model: TaskDetailModel
    let onClose: () -> Void

    @State private var message = ""
    @State private var includeUnstaged = true
    @State private var push = false
    @State private var loading = false
    @State private var error: String?
    @FocusState private var focused: Bool

    private static let gray100 = Color(hex: 0xF3F4F6)

    private var canCommit: Bool { !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !loading }

    var body: some View {
        ZStack {
            Color.black.opacity(0.5).ignoresSafeArea().onTapGesture(perform: onClose)
            dialog
        }
    }

    private var dialog: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                LucideIcon(.gitCommitHorizontal, size: 18, strokeWidth: 1.5).foregroundStyle(Theme.gray300)
                    .padding(8).background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                Text("Commit your changes").font(.system(size: 18, weight: .semibold)).foregroundStyle(Self.gray100)
                    .frame(maxWidth: .infinity, alignment: .leading)
                GlyphButton(glyph: .x, size: 16, color: Theme.gray400, hoverColor: .white, action: onClose)
            }
            .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 12)

            VStack(alignment: .leading, spacing: 16) {
                row("Branch") {
                    HStack(spacing: 6) {
                        LucideIcon(.gitBranch, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                        Text(model.task?.branch ?? "unknown").font(.system(size: 14, design: .monospaced))
                            .foregroundStyle(Theme.gray200)
                    }
                }
                row("Changes") {
                    if let stat = model.stat {
                        HStack(spacing: 8) {
                            Text("\(stat.filesChanged) file\(stat.filesChanged == 1 ? "" : "s")").foregroundStyle(Theme.gray300)
                            Text("+\(stat.insertions)").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.green400)
                            Text("-\(stat.deletions)").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.red400)
                        }
                        .font(.system(size: 14))
                    } else {
                        Text("No changes").font(.system(size: 14)).foregroundStyle(Theme.gray500)
                    }
                }
                HStack(spacing: 12) {
                    Button { includeUnstaged.toggle() } label: {
                        ZStack(alignment: .leading) {
                            Capsule().fill(includeUnstaged ? Theme.blue500 : Theme.white(0.12))
                            Circle().fill(.white).frame(width: 16, height: 16).offset(x: includeUnstaged ? 21 : 3)
                        }
                        .frame(width: 40, height: 22)
                        .animation(.easeOut(duration: 0.15), value: includeUnstaged)
                    }
                    .buttonStyle(.plain)
                    Text("Include unstaged").font(.system(size: 14)).foregroundStyle(Theme.gray300)
                }
                VStack(alignment: .leading, spacing: 8) {
                    label("Commit message")
                    TextEditor(text: $message)
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.gray200)
                        .scrollContentBackground(.hidden)
                        .focused($focused)
                        .frame(height: 60)
                        .padding(.horizontal, 7).padding(.vertical, 8)
                        .overlay(alignment: .topLeading) {
                            if message.isEmpty {
                                Text("Enter a commit message...").font(.system(size: 14)).foregroundStyle(Theme.gray600)
                                    .padding(.horizontal, 12).padding(.vertical, 8).allowsHitTesting(false)
                            }
                        }
                        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(focused ? 0.16 : 0.08)))
                }
                VStack(alignment: .leading, spacing: 8) {
                    label("Next steps")
                    VStack(spacing: 4) {
                        option(.gitCommitHorizontal, "Commit", selected: !push) { push = false }
                        option(.arrowUp, "Commit and push", selected: push) { push = true }
                    }
                }
                if let error {
                    Text(error).font(.system(size: 13)).foregroundStyle(Theme.red400)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(Theme.red500.opacity(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                }
                Button(action: submit) {
                    Group {
                        if loading {
                            SpinningGlyph(glyph: .loaderCircle, size: 16, strokeWidth: 2, active: true)
                        } else {
                            Text("Continue")
                        }
                    }
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(canCommit ? Color(hex: 0x101828) : Theme.gray500)
                    .frame(maxWidth: .infinity).padding(.vertical, 10)
                    .background(canCommit ? Color.white : Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!canCommit)
                .keyboardShortcut(.return, modifiers: .command)
            }
            .padding(.horizontal, 20).padding(.bottom, 20)
        }
        .frame(width: 440)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08)))
        .shadow(color: .black.opacity(0.5), radius: 25, y: 12)
        .onAppear { focused = true }
        .onExitCommand(perform: onClose)
    }

    private func row(_ title: String, @ViewBuilder _ value: () -> some View) -> some View {
        HStack {
            label(title)
            Spacer()
            value()
        }
    }

    private func label(_ text: String) -> some View {
        Text(text).font(.system(size: 14, weight: .medium)).foregroundStyle(Theme.gray300)
    }

    private func option(_ glyph: LucideGlyph, _ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 12) {
                    LucideIcon(glyph, size: 16, strokeWidth: 1.5)
                    Text(title).frame(maxWidth: .infinity, alignment: .leading)
                    if selected { LucideIcon(.check, size: 16, strokeWidth: 2).foregroundStyle(Theme.gray300) }
                }
                .font(.system(size: 14))
                .foregroundStyle(selected ? Self.gray100 : Theme.gray400)
                .padding(.horizontal, 12).padding(.vertical, 10)
                .background(selected ? Theme.white(0.08) : hovered ? Theme.white(0.04) : .clear,
                            in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    /// handleContinue: on success the diff reloads and the dialog closes; a failure stays on screen.
    private func submit() {
        guard canCommit else { return }
        loading = true
        error = nil
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            let failure = await model.commit(message: text, includeUnstaged: includeUnstaged, push: push)
            loading = false
            if let failure { error = failure } else { onClose() }
        }
    }
}
