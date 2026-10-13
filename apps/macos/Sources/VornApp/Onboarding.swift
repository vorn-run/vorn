import AppKit
import SwiftUI
import VornCore
import VornSettings
import VornUI

/// OnboardingModal.tsx: the seven-page welcome guide over the whole window.
struct OnboardingModal: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let settings: SettingsModel

    /// Renders open the plugin card on the first page.
    static var renderMcpOpen = false

    @State private var shown = false
    @State private var bodyHeight: CGFloat = 0
    @State private var forward = true
    @State private var keys: Any?

    private var step: Int { Onboarding.clamp(model.onboardingStep) }
    private var isLast: Bool { step == Onboarding.steps.count - 1 }

    var body: some View {
        GeometryReader { geo in
            modal
                .frame(maxHeight: geo.size.height * 0.85)
                .fixedSize(horizontal: true, vertical: false)
                .scaleEffect(shown ? 1 : 0.96)
                .opacity(shown ? 1 : 0)
                .position(x: geo.size.width / 2, y: geo.size.height / 2)
        }
        .onAppear {
            withAnimation(.timingCurve(0.4, 0, 0.2, 1, duration: 0.18)) { shown = true }
            installKeys()
        }
        .onDisappear(perform: removeKeys)
    }

    private var modal: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                stepBody
                    .padding(.horizontal, 24)
                    .padding(.vertical, 20)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(GeometryReader { Color.clear.preference(key: BodyHeight.self, value: $0.size.height) })
            }
            .scrollIndicators(.never)
            .frame(maxHeight: bodyHeight > 0 ? bodyHeight : nil)
            .onPreferenceChange(BodyHeight.self) { bodyHeight = $0 }
            footer
        }
        .frame(width: 520)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .shadow(color: .black.opacity(0.5), radius: 25, y: 12)
        .contentShape(Rectangle())
        .onTapGesture {}
    }

    // MARK: Header

    private var header: some View {
        let current = Onboarding.steps[step]
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                ForEach(Array(Onboarding.steps.enumerated()), id: \.element.id) { i, s in
                    Button { go(i) } label: {
                        Capsule()
                            .fill(i <= step ? Theme.bronzo : Theme.white(0.08))
                            .frame(width: i == step ? 32 : 16, height: 4)
                            .padding(.vertical, 4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Go to step \(i + 1): \(s.title)")
                }
                Spacer(minLength: 0)
                Text("\(step + 1)/\(Onboarding.steps.count)").font(.ui(11)).foregroundStyle(Theme.gray600)
            }
            .animation(.easeInOut(duration: 0.3), value: step)
            .padding(.bottom, 12)
            HStack(spacing: 12) {
                LucideIcon(Self.icon(current.icon), size: 20)
                    .foregroundStyle(Theme.bronzo)
                    .frame(width: 40, height: 40)
                    .background(Theme.bronzo.opacity(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusXl))
                VStack(alignment: .leading, spacing: 2) {
                    Text(current.title).font(.ui(16, weight: .semibold)).foregroundStyle(.white)
                    Text(current.subtitle).font(.ui(12)).foregroundStyle(Theme.gray500)
                }
            }
            .id(current.id)
            .transition(.asymmetric(insertion: .opacity.combined(with: .offset(x: forward ? 12 : -12)),
                                    removal: .opacity.combined(with: .offset(x: forward ? -12 : 12))))
        }
        .padding(.horizontal, 24)
        .padding(.top, 20)
        .padding(.bottom, 16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .topTrailing) {
            IconButton(.x, size: 16, padding: 4, color: Theme.gray600, hoverColor: Theme.gray300,
                       help: "Close welcome guide") { model.closeOnboarding() }
                .padding(.top, 14)
                .padding(.trailing, 14)
        }
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 12) {
            Hovering { hover in
                Button { model.closeOnboarding() } label: {
                    Text("Skip guide").font(.ui(12)).foregroundStyle(hover ? Theme.gray400 : Theme.gray600)
                }
                .buttonStyle(.plain)
            }
            Spacer(minLength: 0)
            if step > 0 {
                Hovering { hover in
                    Button { go(step - 1) } label: {
                        HStack(spacing: 6) {
                            LucideIcon(.arrowLeft, size: 13)
                            Text("Back").font(.ui(13))
                        }
                        .foregroundStyle(hover ? .white : Theme.gray400)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(hover ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            Hovering { hover in
                Button { isLast ? model.closeOnboarding() : go(step + 1) } label: {
                    HStack(spacing: 6) {
                        Text(isLast ? "Get Started" : "Next").font(.ui(13, weight: .medium))
                        LucideIcon(.arrowRight, size: 13)
                    }
                    .foregroundStyle(.black)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 6)
                    .background(hover ? Theme.bronzoDark : Theme.bronzo, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    // MARK: Body

    @ViewBuilder private var stepBody: some View {
        let id = Onboarding.steps[step].id
        Group {
            switch id {
            case "agents": OnboardingAgentsStep(settings: settings)
            case "projects": projectsStep
            case "sessions": sessionsStep
            case "workspace": workspaceStep
            case "workflows": workflowsStep
            case "shortcuts": shortcutsStep
            default: readyStep
            }
        }
        .id(id)
        .transition(.asymmetric(insertion: .opacity.combined(with: .offset(y: forward ? 8 : -8)),
                                removal: .opacity.combined(with: .offset(y: forward ? -8 : 8))))
    }

    private var projectsStep: some View {
        VStack(alignment: .leading, spacing: 12) {
            Prose([.text("A "), .strong("project"), .text(" is a Git repo. "), .strong("Worktrees"),
                   .text(" let multiple agents work on different branches simultaneously without conflicts.")])
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 12) {
                    LucideIcon(.folderGit2, size: 15).foregroundStyle(Theme.cyan400).padding(.top, 1)
                    Text("my-app/").font(.ui(12, weight: .medium)).foregroundStyle(Theme.gray200)
                }
                VStack(alignment: .leading, spacing: 6) {
                    branchRow("main", icon: Theme.gray500, chip: "default", chipText: Theme.gray600, chipFill: Theme.white(0.04))
                    branchRow("feat/auth", icon: Theme.violet400, chip: "worktree", chipText: Theme.violet400.opacity(0.6),
                              chipFill: Theme.violet400.opacity(0.06))
                    branchRow("fix/api-bug", icon: Theme.amber400, chip: "worktree", chipText: Theme.amber400.opacity(0.6),
                              chipFill: Theme.amber400.opacity(0.06))
                }
                .padding(.leading, 16)
                .overlay(alignment: .leading) { Rectangle().fill(Theme.hairline).frame(width: 1) }
                .padding(.leading, 16)
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .sunken()
            Grid(horizontalSpacing: 6, verticalSpacing: 6) {
                GridRow {
                    featureTile(.squareKanban, "Kanban board")
                    featureTile(.gitBranch, "Auto branch & worktree")
                }
                GridRow {
                    featureTile(.bot, "Launch agents from tasks")
                    featureTile(.listTodo, "Full task lifecycle")
                }
            }
            HStack(spacing: 12) {
                let count = store.projects.count
                if count > 0 {
                    Text("\(count) project\(count > 1 ? "s" : "") configured").font(.ui(12)).foregroundStyle(Theme.gray500)
                } else {
                    OnboardingAccentButton(icon: .folderPlus, label: "Add project") {
                        closeAndOpen { model.overlay = .addProject(editing: nil) }
                    }
                }
                OnboardingLink(icon: .squareKanban, label: "Open Tasks") { closeAndOpen { model.mainViewMode = .tasks } }
            }
        }
    }

    private func branchRow(_ name: String, icon: Color, chip: String, chipText: Color, chipFill: Color) -> some View {
        HStack(spacing: 8) {
            LucideIcon(.gitBranch, size: 11).foregroundStyle(icon)
            Text(name).font(.ui(11)).foregroundStyle(Theme.gray400)
            Text(chip).font(.ui(9)).foregroundStyle(chipText)
                .padding(.horizontal, 6).padding(.vertical, 2)
                .background(chipFill, in: RoundedRectangle(cornerRadius: Theme.radius))
        }
    }

    private func featureTile(_ icon: LucideGlyph, _ text: String) -> some View {
        HStack(spacing: 8) {
            LucideIcon(icon, size: 11).foregroundStyle(Theme.gray500)
            Text(text).font(.ui(11)).foregroundStyle(Theme.gray400).lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .sunken(radius: Theme.radiusMd, border: Theme.white(0.04))
    }

    private var sessionsStep: some View {
        VStack(alignment: .leading, spacing: 16) {
            Prose([.text("Press "), .kbd("⌘N"), .text(" to open the session launcher. Pick an agent, project, and branch — then type an optional prompt to direct the agent.")])
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 8) {
                    ForEach([(LucideGlyph.bot, "Agent"), (.folderGit2, "Project"), (.gitBranch, "Branch")], id: \.1) { icon, label in
                        HStack(spacing: 6) {
                            LucideIcon(icon, size: 12).foregroundStyle(Theme.gray400)
                            Text(label).font(.ui(11)).foregroundStyle(Theme.gray300)
                            LucideIcon(.chevronDown, size: 10).foregroundStyle(Theme.gray600)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                    }
                }
                Text("Type a prompt to direct the agent...").font(.ui(11)).italic().foregroundStyle(Theme.gray600)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.black.opacity(0.3), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                HStack {
                    Text("Sessions can also run on remote hosts via SSH").font(.ui(10)).foregroundStyle(Theme.gray600)
                    Spacer(minLength: 0)
                    HStack(spacing: 4) {
                        LucideIcon(.rocket, size: 10)
                        Text("Launch").font(.ui(11))
                    }
                    .foregroundStyle(Theme.gray400)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 4)
                    .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                }
            }
            .padding(16)
            .sunken()
            OnboardingAccentButton(icon: .rocket, label: "Try it now") {
                closeAndOpen {
                    if let project = actions.activeProject ?? store.projects.first {
                        actions.newSession(project, actions.activeProject == nil ? nil : actions.activeWorktree)
                    } else {
                        model.overlay = .addProject(editing: nil)
                    }
                }
            }
        }
    }

    private var workspaceStep: some View {
        VStack(alignment: .leading, spacing: 16) {
            Prose([.text("View sessions in a multi-agent "), .strong("Grid"), .text(" or focused "), .strong("Tabs"),
                   .text(" layout. Press "), .kbd("⌘J"), .text(" to toggle.")])
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    cardTitle(.layoutGrid, Theme.cyan400, "Grid")
                    Grid(horizontalSpacing: 6, verticalSpacing: 6) {
                        GridRow { mockPane(32); mockPane(32) }
                        GridRow { mockPane(32); mockPane(32) }
                    }
                    Text("Side-by-side comparison").font(.ui(10)).foregroundStyle(Theme.gray600)
                }
                .padding(12)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .sunken()
                VStack(alignment: .leading, spacing: 8) {
                    cardTitle(.columns3, Theme.violet400, "Tabs")
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 4) {
                            ForEach(Array(["Agent 1", "Agent 2", "Agent 3"].enumerated()), id: \.offset) { i, t in
                                Text(t).font(.ui(9))
                                    .foregroundStyle(i == 0 ? Theme.bronzo : Theme.gray600)
                                    .padding(.horizontal, 6).padding(.vertical, 2)
                                    .background(i == 0 ? Theme.bronzo.opacity(0.08) : Theme.white(0.03),
                                                in: RoundedRectangle(cornerRadius: Theme.radius))
                            }
                        }
                        mockPane(48)
                    }
                    Text("Maximum terminal space").font(.ui(10)).foregroundStyle(Theme.gray600)
                }
                .padding(12)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .sunken()
            }
            .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    LucideIcon(.panelRight, size: 12).foregroundStyle(Theme.gray400)
                    Text("Review Panel").font(.ui(11, weight: .medium)).foregroundStyle(Theme.gray300)
                    Text("— click the diff badge on any card").font(.ui(10)).foregroundStyle(Theme.gray600)
                }
                HStack(spacing: 12) {
                    ForEach([(LucideGlyph.fileCode, "File tree"), (.fileDiff, "Syntax diffs"), (.gitCommitHorizontal, "Commit & push")],
                            id: \.1) { icon, label in
                        HStack(spacing: 4) {
                            LucideIcon(icon, size: 10)
                            Text(label).font(.ui(11))
                        }
                        .foregroundStyle(Theme.gray500)
                    }
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .sunken()
            Text("Headless agents appear as compact pills above the grid for background tasks.")
                .font(.ui(11)).foregroundStyle(Theme.gray600)
        }
    }

    private func cardTitle(_ icon: LucideGlyph, _ tint: Color, _ title: String) -> some View {
        HStack(spacing: 6) {
            LucideIcon(icon, size: 11).foregroundStyle(tint)
            Text(title).font(.ui(11, weight: .medium)).foregroundStyle(Theme.gray300)
        }
        .padding(.bottom, 4)
    }

    private func mockPane(_ height: CGFloat) -> some View {
        RoundedRectangle(cornerRadius: Theme.radius)
            .fill(Theme.white(0.04))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.hairline, lineWidth: 1))
            .frame(maxWidth: .infinity)
            .frame(height: height)
    }

    private var workflowsStep: some View {
        VStack(alignment: .leading, spacing: 16) {
            Prose([.text("Build multi-step workflows that launch agents, run scripts, and branch based on conditions. Trigger them manually, on a schedule, or when a task changes status.")])
            VStack(alignment: .leading, spacing: 10) {
                sectionLabel("Trigger types").padding(.bottom, 8)
                ForEach([(LucideGlyph.zap, "Manual", "Run on demand from sidebar"),
                         (.clock, "Scheduled", "Cron or one-time schedule"),
                         (.squareKanban, "Task event", "When a task status changes")], id: \.1) { icon, label, desc in
                    HStack(spacing: 12) {
                        LucideIcon(icon, size: 12).foregroundStyle(Theme.gray400)
                            .frame(width: 24, height: 24)
                            .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radius))
                        VStack(alignment: .leading, spacing: 1) {
                            Text(label).font(.ui(12)).foregroundStyle(Theme.gray300)
                            Text(desc).font(.ui(11)).foregroundStyle(Theme.gray600)
                        }
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .sunken()
            HStack(spacing: 8) {
                ForEach(["Launch Agent", "Run Script", "Condition", "Parallel"], id: \.self) { n in
                    Text(n).font(.ui(11)).foregroundStyle(Theme.gray400)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .sunken(radius: Theme.radiusMd)
                }
            }
            Text("Create workflows from the sidebar or command palette.").font(.ui(11)).foregroundStyle(Theme.gray600)
        }
    }

    /// OnboardingModal.tsx `ESSENTIAL_SHORTCUTS`, described as keyboard-shortcuts.ts does.
    static let essentialShortcuts: [(String, String)] = [
        ("Command palette", "⌘K"), ("New session", "⌘N"), ("View options", "⌘J"),
        ("Toggle sidebar", "⌘B"), ("Keyboard shortcuts", "⌘/"), ("Show all", "⌥1"),
    ]

    private var shortcutsStep: some View {
        VStack(alignment: .leading, spacing: 16) {
            Prose([.text("Press "), .kbd("⌘K"), .text(" to open the command palette — it searches sessions, projects, workflows, and every action in one place.")])
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                ForEach(0..<3, id: \.self) { r in
                    GridRow {
                        ForEach(0..<2, id: \.self) { c in
                            let (label, key) = Self.essentialShortcuts[r * 2 + c]
                            HStack(spacing: 8) {
                                Text(label).font(.ui(12)).foregroundStyle(Theme.gray400)
                                Spacer(minLength: 0)
                                OnboardingKbd(key)
                            }
                        }
                    }
                }
            }
            .padding(12)
            .sunken()
            OnboardingAccentButton(icon: .search, label: "Try Command Palette") {
                closeAndOpen { model.openCommandPalette() }
            }
            Prose([.text("View all shortcuts anytime with "), .kbd("⌘/")], size: 11, color: Theme.gray600)
        }
    }

    private var readyStep: some View {
        VStack(alignment: .leading, spacing: 16) {
            Prose([.text("You are all set. Here are a few more things to explore when you are ready.")])
            VStack(spacing: 6) {
                ForEach([(LucideGlyph.bell, "Enable notifications & sounds", SettingsCategory.notifications),
                         (.monitor, "Set up remote hosts (SSH)", .ssh),
                         (.palette, "Customize appearance", .appearance),
                         (.settings, "Configure agent commands", .agents)], id: \.1) { icon, label, category in
                    exploreRow(icon, label) {
                        closeAndOpen {
                            settings.category = category
                            model.settingsOpen = true
                        }
                    }
                }
            }
            Text("You can reopen this guide anytime from the sidebar or command palette.")
                .font(.ui(11)).foregroundStyle(Theme.gray600)
        }
    }

    private func exploreRow(_ icon: LucideGlyph, _ label: String, action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 12) {
                    LucideIcon(icon, size: 14).foregroundStyle(Theme.gray400)
                        .frame(width: 28, height: 28)
                        .background(Theme.white(hover ? 0.06 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    Text(label).font(.ui(13)).foregroundStyle(Theme.gray300)
                    Spacer(minLength: 0)
                    LucideIcon(.arrowRight, size: 12).foregroundStyle(hover ? Theme.gray400 : Theme.gray600)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(Theme.surfaceSunken, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .background(hover ? Theme.white(0.02) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                    .strokeBorder(Theme.white(hover ? 0.08 : 0.04), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: Navigation

    private func go(_ target: Int) {
        let next = Onboarding.clamp(target)
        guard next != step else { return }
        forward = next > step
        withAnimation(.easeOut(duration: 0.15)) { model.onboardingStep = next }
    }

    /// OnboardingModal.tsx `closeAndOpen`: shuts the guide without remembering it, then runs `action` after its exit.
    private func closeAndOpen(_ action: @escaping @MainActor () -> Void) {
        model.overlay = nil
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(200))
            action()
        }
    }

    private func installKeys() {
        guard keys == nil else { return }
        keys = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [model] event in
            guard model.overlay == .onboarding,
                  event.modifierFlags.intersection([.command, .shift, .option, .control]).isEmpty else { return event }
            switch event.keyCode {
            case 124: forward = true; withAnimation(.easeOut(duration: 0.15)) { model.onboardingStep = Onboarding.next(model.onboardingStep) }
            case 123: forward = false; withAnimation(.easeOut(duration: 0.15)) { model.onboardingStep = Onboarding.previous(model.onboardingStep) }
            default: return event
            }
            return nil
        }
    }

    private func removeKeys() {
        if let keys { NSEvent.removeMonitor(keys) }
        keys = nil
    }

    static func icon(_ name: String) -> LucideGlyph {
        switch name {
        case "Bot": .bot
        case "FolderGit2": .folderGit2
        case "Rocket": .rocket
        case "LayoutDashboard": .layoutDashboard
        case "Workflow": .workflow
        case "Search": .search
        default: .sparkles
        }
    }
}

private struct BodyHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// OnboardingModal.tsx `StepAgents`: which agents are installed, and how to give them the Vorn plugin.
struct OnboardingAgentsStep: View {
    let settings: SettingsModel
    @State private var mcpOpen = OnboardingModal.renderMcpOpen

    private var status: [String: Bool] { settings.installedAgents ?? [:] }
    private var installed: [AgentDefinition] { AgentDefinition.all.filter { status[$0.type.rawValue] == true } }
    private var missing: [AgentDefinition] { AgentDefinition.all.filter { status[$0.type.rawValue] != true } }
    private var setups: [AgentMcpSetup] {
        let agents = Onboarding.mcpAgents(order: AgentMcpSetup.all.map(\.agent)) { status[$0.rawValue] == true }
        return AgentMcpSetup.all.filter { agents.contains($0.agent) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Prose([.text("Vorn works with multiple AI coding agents. We detected which ones you have installed.")])
            if !installed.isEmpty {
                VStack(spacing: 6) {
                    ForEach(installed, id: \.type) { def in
                        HStack(spacing: 12) {
                            AgentIcon(def.type, size: 14)
                                .frame(width: 28, height: 28)
                                .background(def.bg, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                            Text(def.type.displayName).font(.ui(14)).foregroundStyle(Theme.gray200)
                            Spacer(minLength: 0)
                            HStack(spacing: 4) {
                                LucideIcon(.check, size: 10)
                                Text("Ready").font(.ui(11))
                            }
                            .foregroundStyle(Theme.emerald400)
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .sunken(radius: Theme.radiusLg)
                    }
                }
            }
            if !missing.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    sectionLabel("Not detected")
                    ForEach(missing, id: \.type) { def in
                        HStack(spacing: 12) {
                            AgentIcon(def.type, size: 14)
                                .frame(width: 28, height: 28)
                                .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                                .opacity(0.4)
                            Text(def.type.displayName).font(.ui(14)).foregroundStyle(Theme.gray500)
                            Spacer(minLength: 8)
                            Text(def.installCommand).font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.gray600)
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .frame(maxWidth: 180, alignment: .trailing)
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .sunken(radius: Theme.radiusLg, border: Theme.white(0.04))
                    }
                }
            }
            Hovering { hover in
                Button { Task { await settings.detectAgents() } } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.refreshCw, size: 11)
                            .rotationEffect(.degrees(settings.detectingAgents ? 360 : 0))
                            .animation(settings.detectingAgents ? .linear(duration: 1).repeatForever(autoreverses: false) : .default,
                                       value: settings.detectingAgents)
                        Text(settings.detectingAgents ? "Scanning..." : "Re-detect agents").font(.ui(12))
                    }
                    .foregroundStyle(hover && !settings.detectingAgents ? Theme.gray300 : Theme.gray500)
                    .opacity(settings.detectingAgents ? 0.4 : 1)
                }
                .buttonStyle(.plain)
                .disabled(settings.detectingAgents)
            }
            mcpCard
        }
        .task { if settings.installedAgents == nil { await settings.detectAgents() } }
    }

    private var mcpCard: some View {
        VStack(alignment: .leading, spacing: 0) {
            Hovering { hover in
                Button { mcpOpen.toggle() } label: {
                    HStack(spacing: 8) {
                        LucideIcon(.plug, size: 13).foregroundStyle(Theme.bronzo)
                        Text("Install skills & MCP server").font(.ui(12, weight: .medium)).foregroundStyle(Theme.gray200)
                        Spacer(minLength: 0)
                        Text("Recommended").font(.ui(10, weight: .medium)).foregroundStyle(Theme.bronzo)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Theme.bronzo.opacity(0.08), in: RoundedRectangle(cornerRadius: Theme.radius))
                        LucideIcon(mcpOpen ? .chevronDown : .chevronRight, size: 12).foregroundStyle(Theme.gray600)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(hover ? Theme.white(0.02) : .clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            if mcpOpen {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Agents get full context about your projects and tasks — they can launch other agents, pick up tasks, read terminal output, and trigger workflows autonomously.")
                        .font(.ui(12)).foregroundStyle(Theme.gray400).lineSpacing(3)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 8)
                    HStack(spacing: 8) {
                        ForEach(["Launch sessions", "Manage tasks", "Read terminals", "Trigger workflows"], id: \.self) { cap in
                            HStack(spacing: 4) {
                                LucideIcon(.check, size: 8).foregroundStyle(Theme.emerald400.opacity(0.5))
                                Text(cap).font(.ui(10)).foregroundStyle(Theme.gray400)
                            }
                            .padding(.horizontal, 8).padding(.vertical, 2)
                            .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radius))
                        }
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(setups, id: \.agent) { setup in
                            setupBlock(setup)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
                .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
            }
        }
        .sunken(radius: Theme.radiusLg)
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
    }

    private func setupBlock(_ setup: AgentMcpSetup) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                AgentIcon(setup.agent, size: 10)
                    .frame(width: 20, height: 20)
                    .background(AgentDefinition.of(setup.agent)?.bg ?? Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radius))
                sectionLabel(setup.inAgent ? "Run in \(setup.agent.displayName)" : "Run in your terminal", size: 10)
            }
            ForEach(setup.commands, id: \.self) { command in
                HStack(spacing: 8) {
                    ScrollView(.horizontal) {
                        Text(command).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray400)
                            .fixedSize()
                            .textSelection(.enabled)
                    }
                    .scrollIndicators(.never)
                    OnboardingCopyButton(text: command)
                }
                .padding(.leading, 28)
            }
            if let note = setup.note {
                Text(note).font(.ui(10)).foregroundStyle(Theme.gray600)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, 28)
            }
        }
    }
}

/// OnboardingModal.tsx `CopyBtn`: copies a command, and shows a check for 1.5s.
struct OnboardingCopyButton: View {
    let text: String
    @State private var copied = false
    @State private var reset: Task<Void, Never>?

    var body: some View {
        Hovering { hover in
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                copied = true
                reset?.cancel()
                reset = Task { try? await Task.sleep(for: .milliseconds(1500)); if !Task.isCancelled { copied = false } }
            } label: {
                LucideIcon(copied ? .circleCheck : .copy, size: 12)
                    .foregroundStyle(copied ? Theme.emerald400 : hover ? .white : Theme.gray500)
                    .padding(6)
                    .background(Theme.white(hover ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Copy")
        }
        .onDisappear { reset?.cancel() }
    }
}

/// OnboardingModal.tsx `AccentButton`.
struct OnboardingAccentButton: View {
    let icon: LucideGlyph
    let label: String
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(icon, size: 13)
                    Text(label).font(.ui(12, weight: .medium))
                }
                .foregroundStyle(Theme.bronzo)
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .background(Theme.bronzo.opacity(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(hover ? 0.14 : 0.08), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// OnboardingModal.tsx: a quiet text link with an icon ("Open Tasks").
struct OnboardingLink: View {
    let icon: LucideGlyph
    let label: String
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    LucideIcon(icon, size: 11)
                    Text(label).font(.ui(12))
                }
                .foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// OnboardingModal.tsx `Kbd`: a key in the guide's copy.
struct OnboardingKbd: View {
    let key: String
    init(_ key: String) { self.key = key }

    var body: some View {
        Text(key)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(Theme.gray400)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}

/// OnboardingModal.tsx's paragraphs: plain words, highlighted words and keys, wrapped as one line of text.
struct Prose: View {
    enum Run { case text(String), strong(String), kbd(String) }
    let runs: [Run]
    var size: CGFloat = 13
    var color: Color = Theme.gray400

    init(_ runs: [Run], size: CGFloat = 13, color: Color = Theme.gray400) {
        self.runs = runs
        self.size = size
        self.color = color
    }

    private enum Piece: Hashable { case word(String, strong: Bool), key(String) }

    private var pieces: [Piece] {
        runs.flatMap { run -> [Piece] in
            switch run {
            case .kbd(let k): return [.key(k)]
            case .text(let t), .strong(let t):
                let strong = if case .strong = run { true } else { false }
                var words: [Piece] = []
                var current = ""
                for ch in t {
                    current.append(ch)
                    if ch == " " { words.append(.word(current, strong: strong)); current = "" }
                }
                if !current.isEmpty { words.append(.word(current, strong: strong)) }
                return words
            }
        }
    }

    var body: some View {
        ProseFlow(lineSpacing: size * 0.625 - 2) {
            ForEach(Array(pieces.enumerated()), id: \.offset) { _, piece in
                switch piece {
                case .word(let w, let strong):
                    Text(w).font(.ui(size)).foregroundStyle(strong ? Theme.gray200 : color).fixedSize()
                case .key(let k):
                    OnboardingKbd(k).padding(.trailing, 4).fixedSize()
                }
            }
        }
    }
}

/// Lays words out left to right, wrapping at the width, each line centred on its tallest piece.
private struct ProseFlow: Layout {
    var lineSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let lines = arrange(proposal.width ?? .infinity, subviews)
        let width = proposal.width ?? lines.map(\.width).max() ?? 0
        return CGSize(width: width.isFinite ? width : lines.map(\.width).max() ?? 0, height: lines.last.map { $0.y + $0.height } ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for line in arrange(bounds.width, subviews) {
            var x = bounds.minX
            for i in line.range {
                let s = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + line.y + (line.height - s.height) / 2),
                                  proposal: .unspecified)
                x += s.width
            }
        }
    }

    private func arrange(_ width: CGFloat, _ subviews: Subviews) -> [(y: CGFloat, height: CGFloat, width: CGFloat, range: Range<Int>)] {
        var lines: [(y: CGFloat, height: CGFloat, width: CGFloat, range: Range<Int>)] = []
        var x: CGFloat = 0, y: CGFloat = 0, h: CGFloat = 0, start = 0
        for (i, v) in subviews.enumerated() {
            let s = v.sizeThatFits(.unspecified)
            if i > start && x + s.width > width + 3 {
                lines.append((y, h, x, start..<i))
                y += h + lineSpacing
                x = 0; h = 0; start = i
            }
            x += s.width
            h = max(h, s.height)
        }
        if start < subviews.count { lines.append((y, h, x, start..<subviews.count)) }
        return lines
    }
}

/// KeyboardShortcutsPanel.tsx: the row under the shortcuts that opens the welcome guide.
struct WelcomeGuideRow: View {
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack {
                    Text("Show Welcome Guide").font(.ui(13)).foregroundStyle(Theme.bronzo)
                    Spacer(minLength: 0)
                    Text("Feature tour").font(.ui(11)).foregroundStyle(Theme.gray500)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(hover ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.top, 12)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }
}

private func sectionLabel(_ text: String, size: CGFloat = 11) -> some View {
    Text(text.uppercased())
        .font(.ui(size, weight: .medium))
        .tracking(0.5)
        .foregroundStyle(Theme.gray600)
}

private extension View {
    /// The guide's cards: `rounded-lg border border-white/[0.06] bg-surface-sunken`.
    func sunken(radius: CGFloat = Theme.radiusLg, border: Color = Theme.hairline) -> some View {
        background(Theme.surfaceSunken, in: RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(border, lineWidth: 1))
    }
}

/// App.tsx: opens the guide once the config is known, unless this version of it was already seen.
enum OnboardingGate {
    /// Points the guide's close at the config, so dismissing it is remembered as App.tsx does.
    @MainActor
    static func bind(model: AppModel, store: VornStore) {
        model.onOnboardingDone = { [weak store] in
            guard let store, !store.readOnly else { return }
            Task { try? await store.setDefault("hasSeenOnboarding", .number(Double(Onboarding.version))) }
        }
    }

    /// Waits for the connection, reads `defaults.hasSeenOnboarding`, and opens the guide when it is behind.
    /// Returns whether it opened.
    @MainActor @discardableResult
    static func openIfUnseen(model: AppModel, store: VornStore, timeout: Duration = .seconds(30)) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while store.phase != .connected {
            guard ContinuousClock.now < deadline, !Task.isCancelled else { return false }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard let config = try? await store.client.call("config:load"),
              Onboarding.shouldOpen(hasSeen: config["defaults"]?["hasSeenOnboarding"]) else { return false }
        model.openOnboarding()
        return true
    }
}
