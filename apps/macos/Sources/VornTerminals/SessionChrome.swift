import SwiftUI
import VornCore
import VornUI

/// `formatRelativeTime`: "Just now", "5m ago", "3h ago", "2d ago", then "Mar 4".
public func formatRelativeTime(_ ms: Double, now: Date = Date()) -> String {
    let date = Date(timeIntervalSince1970: ms / 1000)
    let mins = Int(now.timeIntervalSince(date) / 60)
    if mins < 1 { return "Just now" }
    if mins < 60 { return "\(mins)m ago" }
    let hours = mins / 60
    if hours < 24 { return "\(hours)h ago" }
    let days = hours / 24
    if days < 30 { return "\(days)d ago" }
    let f = DateFormatter()
    f.locale = Locale(identifier: "en_US")
    f.dateFormat = "MMM d"
    return f.string(from: date)
}

/// `shortenCwd`: the last two parts of a path.
func shortenCwd(_ cwd: String?) -> String? {
    guard let cwd else { return nil }
    let parts = cwd.split(separator: "/")
    if parts.isEmpty { return "/" }
    if parts.count <= 2 { return "/" + parts.joined(separator: "/") }
    return parts.suffix(2).joined(separator: "/")
}

/// The row beneath a terminal (SessionComposer.tsx): why it ended, or a place to type in a shell.
struct SessionComposer: View {
    let session: TerminalSession
    let ended: EndedInfo?
    let compact: Bool
    let actions: CardActions

    var body: some View {
        if let ended {
            EndedStrip(session: session, ended: ended, compact: compact, actions: actions)
        } else if session.agentType == .shell {
            IntentBar(session: session, compact: compact, actions: actions)
        }
    }
}

/// EndedStrip.tsx: a pane with nothing running behind it says why, and offers the way back.
struct EndedStrip: View {
    let session: TerminalSession
    let ended: EndedInfo
    let compact: Bool
    let actions: CardActions
    @State private var warningLeft = false

    private var isShell: Bool { session.agentType == .shell }
    private var env: RestoreEnvironment? { ended.environment }
    private var worktreeGone: Bool { env?.worktreeMissing == true }
    private var moved: Bool { env?.headMoved == true }
    private var finding: String? { worktreeGone ? "the worktree is gone" : moved ? "the branch moved" : nil }
    private var warning: Bool { moved && !worktreeGone && !warningLeft }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Rectangle().fill(finding == nil ? Theme.inkGhost : Theme.red400).frame(width: 7, height: 7)
                summary
                    .font(.ui(11))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if !worktreeGone && !warning {
                    StripButton(isShell ? "New shell here" : "Resume", glyph: isShell ? .terminal : .play) {
                        actions.resume(session.id)
                    }
                }
                IconButton(.x, size: 11, padding: 4, radius: Theme.radius, color: Theme.inkFaint, hoverColor: Theme.ink,
                           hoverFill: Theme.white(0.08), help: "Close this pane") { actions.close(session.id) }
            }
            .padding(.vertical, 6)
            .padding(.leading, 12)
            .padding(.trailing, 8)
            if showChecks, let env { checks(env) }
            if worktreeGone && !compact { goneSummary }
            if warning { warningBlock }
        }
        .background(Theme.surfaceRaised)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private var summary: Text {
        var t = Text("Ended").foregroundColor(Theme.ink)
            + Text(" · \(ended.cause) · \(finding ?? formatRelativeTime(ended.at))").foregroundColor(Theme.inkFaint)
        if !ended.replayed { t = t + Text(" · nothing of its screen was kept").foregroundColor(Theme.inkFaint) }
        if ended.partial { t = t + Text(" · the last moments were not recorded").foregroundColor(Theme.inkFaint) }
        if !compact, isShell, let cwd = shortenCwd(ended.cwd) { t = t + Text(" · \(cwd)").foregroundColor(Theme.inkFaint) }
        return t
    }

    private var showChecks: Bool {
        !compact && env != nil && !isShell && (ended.reason == .machineRestarted || finding != nil)
    }

    private func checks(_ env: RestoreEnvironment) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            let tree = shortenCwd(session.worktreePath ?? session.projectPath) ?? ""
            check(!worktreeGone, "worktree", tree + (worktreeGone ? " — missing" : ""))
            if !worktreeGone {
                check(env.branch.actual == nil || env.branch.recorded == env.branch.actual, "branch",
                      env.branch.actual ?? env.branch.recorded ?? "—")
                if let recorded = env.head.recorded {
                    check(!moved, "HEAD", moved && env.head.actual != nil
                          ? "was \(recorded.prefix(8)) · now \(env.head.actual!.prefix(8))"
                          : "\(recorded.prefix(8)) · unchanged")
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }

    private func check(_ ok: Bool, _ label: String, _ value: String) -> some View {
        HStack(spacing: 8) {
            Text(ok ? "✓" : "✗").foregroundStyle(ok ? Theme.inkFaint : Theme.red400)
            Text(label).foregroundStyle(Theme.inkFaint).frame(width: 56, alignment: .leading)
            Text(value).foregroundStyle(ok ? Theme.inkSecondary : Theme.ink).lineLimit(1)
        }
        .font(.ui(11))
    }

    private var goneSummary: some View {
        VStack(alignment: .leading, spacing: 4) {
            row("agent", session.agentType.rawValue)
            if let branch = session.branch { row("branch", branch) }
            if let name = session.displayName { row("last turn", name) }
            row("ran", formatSpan(ended.at - session.createdAt))
            Text("Cleaned up after it merged, most likely. Nothing to resume into. The conversation is still readable, and this pane can be closed.")
                .foregroundStyle(Theme.inkFaint)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4)
        }
        .font(.ui(11))
        .foregroundStyle(Theme.inkSecondary)
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(spacing: 8) {
            Text(label).foregroundStyle(Theme.inkFaint).frame(width: 56, alignment: .leading)
            Text(value).lineLimit(1)
        }
    }

    private func formatSpan(_ ms: Double) -> String {
        let mins = max(0, Int(ms / 60_000))
        return mins < 60 ? "\(mins)m" : "\(mins / 60)h \(mins % 60)m"
    }

    private var warningBlock: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Something changed the tree while you were away. Resuming would put an agent back into a conversation about a tree that no longer matches it.")
                .font(.ui(11))
                .foregroundStyle(Theme.inkSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 4) {
                StripButton("Resume anyway", glyph: .play) { actions.resume(session.id) }
                StripButton("Show what changed") { actions.browseFiles(session.id) }
                StripButton("Leave it") { warningLeft = true }
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }
}

/// `px-2 py-1 text-[11px] text-ink-secondary hover:bg-white/[0.08]`, an optional 11pt glyph.
struct StripButton: View {
    let label: String
    let glyph: LucideGlyph?
    let action: () -> Void

    init(_ label: String, glyph: LucideGlyph? = nil, action: @escaping () -> Void) {
        self.label = label
        self.glyph = glyph
        self.action = action
    }

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    if let glyph { LucideIcon(glyph, size: 11) }
                    Text(label)
                }
                .font(.ui(11))
                .foregroundStyle(hover ? Theme.ink : Theme.inkSecondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.08) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .fixedSize()
    }
}

/// IntentBar.tsx: a shell's composer, a command run in the pane or a prompt that starts an agent.
struct IntentBar: View {
    let session: TerminalSession
    let compact: Bool
    let actions: CardActions
    @State private var value = ""
    @State private var promptMode = false
    @State private var historyIndex: Int?
    @FocusState private var focused: Bool
    @Environment(\.terminalSnapshot) private var snapshot

    private var label: String { promptMode ? "Describe a task" : "Type a command" }
    private var placeholder: String { promptMode || !value.isEmpty ? label : "Type a command or send a prompt for the agent" }
    private var cwdLabel: String {
        let cwd = session.shellCwd ?? session.worktreePath ?? session.projectPath
        return cwd.split(separator: "/").last.map(String.init) ?? session.projectName
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                Button { promptMode.toggle() } label: {
                    Text(promptMode ? "◇" : "❯")
                        .font(.system(size: 13, design: .monospaced))
                        .foregroundStyle(focused ? Theme.gray300 : Theme.gray600)
                }
                .buttonStyle(.plain)
                .help("\(promptMode ? "Prompt" : "Command") — ⌘I to switch")
                ZStack(alignment: .leading) {
                    if value.isEmpty {
                        Text(placeholder).foregroundStyle(Theme.gray700).allowsHitTesting(false)
                    }
                    if !snapshot {
                        TextField("", text: $value)
                            .textFieldStyle(.plain)
                            .foregroundStyle(Theme.gray200)
                            .focused($focused)
                            .onSubmit(submit)
                            .onKeyPress(.upArrow) { recall(-1) }
                            .onKeyPress(.downArrow) { recall(1) }
                            .onKeyPress(characters: ["i"], phases: .down) { press in
                                guard press.modifiers.contains(.command) else { return .ignored }
                                promptMode.toggle()
                                return .handled
                            }
                    }
                }
                .font(promptMode ? .ui(13).italic() : .system(size: 13, design: .monospaced))
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: 19)
            .padding(.trailing, 8)
            .padding(.top, compact ? 4 : 6)
            .padding(.bottom, 4)
            if !compact {
                HStack(spacing: 8) {
                    HStack(spacing: 4) {
                        LucideIcon(.folder, size: 9, strokeWidth: 1.5)
                        Text(cwdLabel).font(.system(size: 10, design: .monospaced)).lineLimit(1).frame(maxWidth: 200, alignment: .leading)
                    }
                    Spacer(minLength: 0)
                    Text("↑ history")
                    if promptMode {
                        Text("↵ launch")
                        Text("⌘I command")
                    } else {
                        Text("⇥ complete")
                        Text("↵ run")
                    }
                }
                .font(.ui(10))
                .foregroundStyle(Theme.gray700)
                .padding(.trailing, 8)
                .padding(.bottom, 4)
                .opacity(focused || !value.isEmpty ? 1 : 0)
            }
        }
        .padding(.leading, 12)
        .background(focused ? Theme.white(0.02) : .clear)
        .background(Theme.surfaceSunken)
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.05)).frame(height: 1) }
        .animation(.easeOut(duration: 0.15), value: focused)
    }

    private func submit() {
        let text = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        IntentHistory.shared.add(text, for: session.id)
        historyIndex = nil
        value = ""
        if promptMode { actions.prompt(session.id, text) } else { actions.write(session.id, text + "\r") }
    }

    private func recall(_ step: Int) -> KeyPress.Result {
        let entries = IntentHistory.shared.entries(for: session.id)
        guard !entries.isEmpty else { return .ignored }
        let next = (historyIndex ?? entries.count) + step
        guard next >= 0 else { return .handled }
        if next >= entries.count {
            historyIndex = nil
            value = ""
        } else {
            historyIndex = next
            value = entries[next]
        }
        return .handled
    }
}

/// What each shell's composer has sent, oldest first.
@MainActor
final class IntentHistory {
    static let shared = IntentHistory()
    private var byID: [String: [String]] = [:]

    func add(_ text: String, for id: String) {
        var list = byID[id, default: []].filter { $0 != text }
        list.append(text)
        byID[id] = Array(list.suffix(100))
    }

    func entries(for id: String) -> [String] { byID[id] ?? [] }
}

/// FocusedNavHint.tsx: "2 / 5" with previous and next, when there is somewhere to go.
public struct FocusedNavHint: View {
    let index: Int
    let total: Int
    let onPrevious: () -> Void
    let onNext: () -> Void

    public init(index: Int, total: Int, onPrevious: @escaping () -> Void, onNext: @escaping () -> Void) {
        self.index = index
        self.total = total
        self.onPrevious = onPrevious
        self.onNext = onNext
    }

    public var body: some View {
        if total >= 2 {
            HStack(spacing: 4) {
                (Text("\(index + 1)") + Text(" / ").foregroundColor(Theme.gray700) + Text("\(total)"))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(Theme.gray500)
                IconButton(.chevronLeft, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink,
                           hoverColor: Theme.ink, hoverFill: Theme.white(0.10), action: onPrevious)
                    .tooltip("Previous", shortcut: "⌘[")
                IconButton(.chevronRight, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink,
                           hoverColor: Theme.ink, hoverFill: Theme.white(0.10), action: onNext)
                    .tooltip("Next", shortcut: "⌘]")
            }
            .fixedSize()
        }
    }
}
