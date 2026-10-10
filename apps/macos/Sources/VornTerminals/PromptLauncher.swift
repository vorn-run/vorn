import SwiftUI
import VornCore
import VornUI

/// The composer shown on an empty grid (PromptLauncher.tsx, inline mode):
/// describe a task, pick project and agent, Enter launches a session.
public struct PromptLauncher: View {
    let projects: [Project]
    let activeProject: String?
    let defaultAgent: AgentType
    let branchFor: (Project) -> String?
    let isGitRepo: (Project) -> Bool
    let launch: (CreateTerminalPayload) async throws -> Void

    @State private var prompt = ""
    @AppStorage("launch.project") private var savedProject = ""
    @AppStorage("launch.agent") private var savedAgent = ""
    @State private var launchError: String?
    @State private var launching = false
    @State private var tip: Tip
    @Environment(\.terminalSnapshot) private var snapshot
    @FocusState private var focused: Bool

    public init(projects: [Project], activeProject: String?, defaultAgent: AgentType,
                branchFor: @escaping (Project) -> String? = { _ in nil },
                isGitRepo: @escaping (Project) -> Bool = { _ in false },
                tip: Int = 1,
                launch: @escaping (CreateTerminalPayload) async throws -> Void) {
        _tip = State(initialValue: Tip.all[min(max(tip, 0), Tip.all.count - 1)])
        self.projects = projects
        self.activeProject = activeProject
        self.defaultAgent = defaultAgent
        self.branchFor = branchFor
        self.isGitRepo = isGitRepo
        self.launch = launch
    }

    private var project: Project? {
        let name = activeProject ?? savedProject
        return projects.first { $0.name == name }
    }

    private var agent: AgentType {
        savedAgent.isEmpty ? defaultAgent : AgentType(rawValue: savedAgent)
    }

    public var body: some View {
        VStack(spacing: 0) {
            VornLogo(height: 32)
                .opacity(0.5)
                .padding(.bottom, 24)
            box
            hint.padding(.top, 8)
        }
        .frame(maxWidth: 800)
        .padding(.horizontal, 16)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear { if !snapshot { tip = Tip.all.randomElement()! } }
    }

    /// `rounded-xl border border-white/[0.06] bg-surface-raised`: a three-row textarea
    /// (`pt-4 pb-12`, 124pt) with the settings bar laid over its bottom.
    private var box: some View {
        ZStack(alignment: .topLeading) {
            input
                .padding(.horizontal, 16)
                .padding(.top, 16)
            VStack(spacing: 0) {
                Spacer(minLength: 0)
                Rectangle().fill(Theme.white(0.04)).frame(height: 1)
                settingsBar
                    .padding(.horizontal, 12)
                    .frame(height: 42)
            }
        }
        .padding(1)
        .frame(height: 132)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.hairline, lineWidth: 1))
        .contentShape(Rectangle())
        .onTapGesture { focused = true }
    }

    /// `text-sm` with 20pt lines, three of them visible.
    @ViewBuilder private var input: some View {
        ZStack(alignment: .topLeading) {
            if prompt.isEmpty {
                Text("Describe your task...")
                    .font(.ui(14))
                    .foregroundStyle(Theme.gray600)
                    .frame(height: 20)
                    .allowsHitTesting(false)
            }
            if !snapshot {
                TextField("", text: $prompt, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.ui(14))
                    .lineSpacing(3)
                    .foregroundStyle(Theme.gray200)
                    .lineLimit(1...3)
                    .padding(.top, 1.5)
                    .focused($focused)
                    .onKeyPress(.return, phases: .down) { press in
                        if press.modifiers.contains(.shift) { return .ignored }
                        submit()
                        return .handled
                    }
                    .onAppear { focused = true }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: 60, alignment: .topLeading)
    }

    private var settingsBar: some View {
        HStack(spacing: 4) {
            picker {
                ForEach(projects) { p in
                    Button(p.name) { savedProject = p.name }
                }
                if projects.isEmpty { Text("No projects") }
            } label: {
                Chip(tint: project == nil ? Theme.gray500 : Theme.gray300) {
                    LauncherProjectIcon(project: project)
                    ChipText(project?.name ?? "Select project", maxWidth: 140)
                }
            }

            picker {
                ForEach(AgentType.agents, id: \.self) { a in
                    Button(a.displayName) { savedAgent = a.rawValue }
                }
            } label: {
                Chip(tint: Theme.gray400) {
                    AgentIcon(agent, size: 14)
                    ChipText(agent.displayName)
                }
            }

            if agent.choosesModel {
                Chip(tint: Theme.gray500) {
                    LucideIcon(.cpu, size: 13, strokeWidth: 1.75)
                    ChipText("Default", maxWidth: 170)
                }
            }

            if let project, isGitRepo(project) {
                Chip(tint: Theme.gray600, hoverTint: Theme.gray400, hoverFill: Theme.white(0.04)) {
                    LucideIcon(.folderGit2, size: 13, strokeWidth: 1.5)
                }
                Chip(tint: Theme.gray400) {
                    LucideIcon(.gitBranch, size: 12)
                    ChipText(branchFor(project) ?? "branch", maxWidth: 100)
                }
            }

            Spacer(minLength: 0)

            Button(action: submit) {
                LucideIcon(.arrowUp, size: 14, strokeWidth: 2.5)
                    .foregroundStyle(project == nil ? Theme.gray600 : Theme.surfaceBase)
                    .padding(6)
                    .background(Circle().fill(project == nil ? Theme.white(0.06) : Theme.ink))
            }
            .buttonStyle(.plain)
            .disabled(project == nil || launching)
            .help("Launch (Enter)")
        }
    }

    /// A chip that opens a menu; just the chip when drawn offscreen.
    @ViewBuilder
    private func picker<Items: View, Label: View>(@ViewBuilder _ items: () -> Items,
                                                  @ViewBuilder label: () -> Label) -> some View {
        if snapshot {
            label()
        } else {
            Menu(content: items, label: label)
                .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()
        }
    }

    /// `text-[11px] text-gray-600 mt-2 text-center`, 1.5 line height.
    @ViewBuilder private var hint: some View {
        if project == nil {
            Text("Select a project to get started")
                .font(.ui(11))
                .foregroundStyle(Theme.gray600)
                .frame(height: 16.5)
        } else if let launchError {
            Text(launchError).font(.ui(12)).foregroundStyle(Theme.red400).frame(height: 16)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            HStack(spacing: 6) {
                LucideIcon(.lightbulb, size: 11).foregroundStyle(Theme.inkFaint)
                if let s = tip.shortcut {
                    Text(s)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Theme.gray500)
                        .frame(height: 15)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: 4))
                }
                Text(tip.text)
                    .font(.ui(11))
                    .foregroundStyle(Theme.gray600)
                    .frame(height: 16.5)
            }
        }
    }

    private func submit() {
        guard let project, !launching else { return }
        launching = true
        launchError = nil
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        var payload = CreateTerminalPayload(agentType: agent, projectName: project.name, projectPath: project.path)
        if !text.isEmpty { payload.initialPrompt = text }
        Task {
            do {
                try await launch(payload)
                prompt = ""
            } catch {
                launchError = error.localizedDescription
            }
            launching = false
        }
    }
}

/// `flex items-center gap-1.5 px-2 py-1 rounded-md text-xs` with a trailing ChevronDown 10.
struct Chip<Content: View>: View {
    let tint: Color
    var hoverTint: Color?
    var hoverFill = Theme.white(0.06)
    @ViewBuilder let content: () -> Content

    var body: some View {
        Hovering { hover in
            HStack(spacing: 6) {
                content()
                LucideIcon(.chevronDown, size: 10)
            }
            .font(.ui(12))
            .foregroundStyle(hover ? hoverTint ?? tint : tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? hoverFill : .clear))
            .contentShape(Rectangle())
        }
    }
}

/// The composer's project icon: its Lucide icon (else Folder) at 13pt, in the
/// project's colour or the chip's.
struct LauncherProjectIcon: View {
    let project: Project?

    var body: some View {
        let icon = LucideIcon(project?.icon.flatMap(Lucide.init(componentName:)) ?? .folder, size: 13)
        if let color = project?.iconColor.flatMap(Color.init(css:)) {
            icon.foregroundStyle(color)
        } else {
            icon
        }
    }
}

/// A chip's label: one 16pt line, truncated at `maxWidth`.
struct ChipText: View {
    let text: String
    var maxWidth: CGFloat?

    init(_ text: String, maxWidth: CGFloat? = nil) {
        self.text = text
        self.maxWidth = maxWidth
    }

    var body: some View {
        CapWidth(max: maxWidth ?? .infinity) {
            Text(text).lineLimit(1).truncationMode(.tail).frame(height: 16)
        }
    }
}

/// `max-w-[…]`: offers the child at most `max`, and takes only the width it uses.
struct CapWidth: Layout {
    let max: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = Swift.min(proposal.width ?? .infinity, max)
        return subviews.first?.sizeThatFits(ProposedViewSize(width: width, height: proposal.height)) ?? .zero
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
    }
}

/// The composer's rotating tips (tips-data.ts).
struct Tip {
    let text: String
    let shortcut: String?

    static let all: [Tip] = [
        Tip(text: "Open the command palette to quickly launch agents or switch sessions", shortcut: "⌘K"),
        Tip(text: "Toggle the sidebar to focus on your grid", shortcut: "⌘B"),
        Tip(text: "View all keyboard shortcuts", shortcut: "⌘/"),
        Tip(text: "Cycle between agent cards", shortcut: "⌘] / ⌘["),
        Tip(text: "Double-click a card title to rename it inline", shortcut: nil),
        Tip(text: "Create a worktree to give each agent an isolated working directory", shortcut: nil),
        Tip(text: "Jump directly to any card by its position", shortcut: "⌘1\u{2013}⌘9"),
        Tip(text: "Use status filters to focus on running or waiting agents", shortcut: "⌥1\u{2013}⌥5"),
        Tip(text: "Double-click the empty grid area to quick-launch a session with your default agent", shortcut: nil),
        Tip(text: "Right-click a card for quick actions: rename, launch new session, or close", shortcut: nil),
        Tip(text: "Right-click the empty grid area to launch a new session or worktree session", shortcut: nil),
        Tip(text: "Launch a worktree session from the Worktrees section in the sidebar", shortcut: nil),
        Tip(text: "Set up Workflows in the sidebar to launch multi-agent setups with one click", shortcut: nil),
        Tip(text: "Click the diff badge on a card to review all changes an agent has made", shortcut: nil),
        Tip(text: "Drag cards to reorder them when in manual sort mode", shortcut: nil),
        Tip(text: "Add remote hosts in Settings to run agents on other machines via SSH", shortcut: nil),
        Tip(text: "Resume previous sessions from the clock icon in the top toolbar", shortcut: nil),
        Tip(text: "Open Settings to customize font size, default agent, and notifications", shortcut: "⌘,"),
    ]
}
