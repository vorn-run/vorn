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
    @State private var tip = Tip.all[1]
    @Environment(\.terminalSnapshot) private var snapshot
    @FocusState private var focused: Bool

    public init(projects: [Project], activeProject: String?, defaultAgent: AgentType,
                branchFor: @escaping (Project) -> String? = { _ in nil },
                isGitRepo: @escaping (Project) -> Bool = { _ in false },
                launch: @escaping (CreateTerminalPayload) async throws -> Void) {
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
    }

    private var box: some View {
        VStack(spacing: 0) {
            ZStack(alignment: .topLeading) {
                if prompt.isEmpty {
                    Text("Describe your task...")
                        .font(.ui(14))
                        .foregroundStyle(Theme.gray600)
                        .allowsHitTesting(false)
                }
                TextField("", text: $prompt, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.ui(14))
                    .foregroundStyle(Theme.gray200)
                    .lineLimit(3...12)
                    .focused($focused)
                    .onKeyPress(.return, phases: .down) { press in
                        if press.modifiers.contains(.shift) { return .ignored }
                        submit()
                        return .handled
                    }
            }
            .padding(.horizontal, 16)
            .padding(.top, 16)
            .frame(minHeight: 76, alignment: .topLeading)
            Spacer(minLength: 0)
            settingsBar
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
        }
        .frame(minHeight: 124)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.hairline, lineWidth: 1))
        .contentShape(Rectangle())
        .onTapGesture { focused = true }
    }

    private var settingsBar: some View {
        HStack(spacing: 4) {
            Menu {
                ForEach(projects) { p in
                    Button(p.name) { savedProject = p.name }
                }
                if projects.isEmpty { Text("No projects") }
            } label: {
                Chip(tint: project == nil ? Theme.gray500 : Theme.gray300) {
                    ProjectIcon(icon: project?.icon, color: project?.iconColor, size: 13)
                    Text(project?.name ?? "Select project").lineLimit(1).frame(maxWidth: 140)
                        .fixedSize(horizontal: true, vertical: false)
                }
            }
            .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()

            Menu {
                ForEach(AgentType.agents, id: \.self) { a in
                    Button(a.displayName) { savedAgent = a.rawValue }
                }
            } label: {
                Chip(tint: Theme.gray400) {
                    AgentIcon(agent, size: 14)
                    Text(agent.displayName)
                }
            }
            .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()

            Chip(tint: Theme.gray400) {
                LucideIcon(.cpu, size: 13, strokeWidth: 1.75).foregroundStyle(Theme.gray500)
                Text("Default").foregroundStyle(Theme.gray500)
            }

            if let project, isGitRepo(project) {
                Chip(tint: Theme.gray600) {
                    LucideIcon(.folderGit2, size: 13, strokeWidth: 1.5)
                }
                Chip(tint: Theme.gray400) {
                    LucideIcon(.gitBranch, size: 12)
                    Text(branchFor(project) ?? "branch").lineLimit(1).frame(maxWidth: 100)
                        .fixedSize(horizontal: true, vertical: false)
                }
            }

            Spacer(minLength: 0)

            Button(action: submit) {
                LucideIcon(.arrowUp, size: 14)
                    .foregroundStyle(project == nil ? Theme.gray600 : Theme.surfaceBase)
                    .padding(6)
                    .background(Circle().fill(project == nil ? Theme.white(0.06) : Theme.ink))
            }
            .buttonStyle(.plain)
            .disabled(project == nil || launching)
            .help("Launch (Enter)")
        }
    }

    @ViewBuilder private var hint: some View {
        if project == nil {
            Text("Select a project to get started")
                .font(.ui(11))
                .foregroundStyle(Theme.gray600)
        } else if let launchError {
            Text(launchError).font(.ui(12)).foregroundStyle(Theme.red400)
        } else {
            HStack(spacing: 6) {
                LucideIcon(.lightbulb, size: 11).foregroundStyle(Theme.inkFaint)
                if let s = tip.shortcut { Kbd(s) }
                Text(tip.text)
                    .onAppear { if !snapshot { tip = Tip.all.randomElement()! } }
            }
            .font(.ui(11))
            .foregroundStyle(Theme.gray600)
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
    @ViewBuilder let content: () -> Content

    var body: some View {
        Hovering { hover in
            HStack(spacing: 6) {
                content()
                LucideIcon(.chevronDown, size: 10)
            }
            .font(.ui(12))
            .foregroundStyle(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(height: 24)
            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? Theme.white(0.06) : .clear))
            .contentShape(Rectangle())
        }
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
