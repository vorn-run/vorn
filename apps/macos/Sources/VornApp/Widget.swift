import AppKit
import Observation
import SwiftUI
import VornCore
import VornUI

/// Widget.tsx: the widget's live state, its rows from the store and the permission requests vornd holds open.
@MainActor
@Observable
final class WidgetFeed {
    private(set) var queue = PermissionQueue()
    var mode: WidgetViewMode = .full

    @ObservationIgnored private let store: VornStore
    @ObservationIgnored private var listen: Task<Void, Never>?
    /// A request arrived (index.ts shows the widget then, when the window is not in front).
    @ObservationIgnored var onRequest: () -> Void = {}

    init(store: VornStore) {
        self.store = store
    }

    /// `WidgetAgentInfo` for each live session, in the order vornd lists them.
    var agents: [WidgetAgent] { store.sessions.map(WidgetAgent.init) }

    /// widget-preload.ts: follows `widget:permission-request` and `widget:permission-cancelled`.
    func start() {
        guard listen == nil else { return }
        let client = store.client
        listen = Task { [weak self] in
            for await event in await client.events(["widget:permission-request", "widget:permission-cancelled"]) {
                guard case .notification(let method, let params) = event, let self else { continue }
                self.apply(method, params)
            }
        }
    }

    func stop() {
        listen?.cancel()
        listen = nil
    }

    func apply(_ method: String, _ params: JSONValue) {
        switch method {
        case "widget:permission-request":
            guard let request = PermissionRequest(params) else { return }
            queue.add(request)
            onRequest()
        case "widget:permission-cancelled":
            if let id = params.stringValue { queue.remove(id) }
        default: break
        }
    }

    /// Widget.tsx `respondPermission`: answers vornd (`permission:resolve`) and drops the card.
    func respond(_ requestId: String, allow: Bool, updatedPermissions: [JSONValue]? = nil, updatedInput: JSONValue? = nil) {
        queue.remove(requestId)
        let params = PermissionQueue.resolveParams(requestId, allow: allow, updatedPermissions: updatedPermissions,
                                                   updatedInput: updatedInput)
        let client = store.client
        Task { try? await client.call("permission:resolve", params) }
    }
}

/// Widget.tsx: the widget's content, the full list or the compact pill.
struct WidgetView: View {
    let feed: WidgetFeed
    let agents: [WidgetAgent]
    var onFocus: (String) -> Void = { _ in }
    var onShowApp: () -> Void = {}
    var onHide: () -> Void = {}
    var onMode: (WidgetViewMode) -> Void = { _ in }

    var body: some View {
        Group {
            switch feed.mode {
            case .full: full
            case .compact: compact
            }
        }
        .environment(\.colorScheme, .dark)
    }

    private var counts: WidgetCounts { WidgetCounts(agents) }

    private var full: some View {
        VStack(spacing: 0) {
            HStack {
                HStack(spacing: 8) {
                    appButton("Vorn", size: 13)
                    if counts.total > 0 {
                        Text("\(counts.running)/\(counts.total)").font(.ui(11)).foregroundStyle(Theme.white(0.4))
                    }
                }
                Spacer(minLength: 0)
                HStack(spacing: 2) {
                    WidgetGlyphButton(size: 22, glyph: 12, radius: 5, color: 0.35, hoverFill: 0.08, help: "Collapse",
                                      action: { onMode(.compact) }) { WidgetGlyph.minus }
                    WidgetGlyphButton(size: 22, glyph: 12, radius: 5, color: 0.35, hoverFill: 0.08, help: "Hide widget",
                                      action: onHide) { WidgetGlyph.close(inset: 3, of: 12) }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            if agents.isEmpty {
                Text("No active agents").font(.ui(12)).foregroundStyle(Theme.white(0.3))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                    .padding(.horizontal, 12)
            } else {
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(agents) { agent in
                            WidgetRow(agent: agent) { onFocus(agent.id) }
                            let cards = feed.queue.forTerminal(agent.id)
                            if !cards.isEmpty {
                                VStack(spacing: 6) {
                                    ForEach(cards) { request in
                                        PermissionCardView(request: request, feed: feed)
                                    }
                                }
                                .padding(.horizontal, 8)
                                .padding(.vertical, 6)
                                .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
                            }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .scrollIndicators(.never)
                .frame(maxHeight: 320)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(width: WidgetViewMode.full.size.width)
        .background(Theme.surfaceOverlay.opacity(0.96), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .frame(maxHeight: WidgetViewMode.full.size.height, alignment: .top)
    }

    private var compact: some View {
        HStack(spacing: 8) {
            appButton("V_", size: 11)
            HStack(spacing: 6) {
                badge(.running, counts.running)
                badge(.waiting, counts.waiting)
                badge(.error, counts.error)
                if counts.total == 0 {
                    Text("0").font(.ui(10)).foregroundStyle(Theme.white(0.3))
                }
            }
            Spacer(minLength: 0)
            HStack(spacing: 2) {
                WidgetGlyphButton(size: 20, glyph: 10, radius: 4, color: 0.3, hoverFill: 0.1, help: "Expand panel",
                                  action: { onMode(.full) }) { WidgetGlyph.expand }
                WidgetGlyphButton(size: 20, glyph: 10, radius: 4, color: 0.3, hoverFill: 0.1, help: "Hide widget",
                                  action: onHide) { WidgetGlyph.close(inset: 2.5, of: 10) }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(width: WidgetViewMode.compact.size.width, height: WidgetViewMode.compact.size.height)
        .background(Theme.surfaceOverlay.opacity(0.96), in: RoundedRectangle(cornerRadius: 20))
        .overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 20))
    }

    @ViewBuilder private func badge(_ status: AgentStatus, _ count: Int) -> some View {
        if count > 0 {
            HStack(spacing: 3) {
                StatusDot(status: status, size: 6)
                Text("\(count)").font(.ui(10)).foregroundStyle(Theme.white(0.6))
            }
        }
    }

    private func appButton(_ label: String, size: CGFloat) -> some View {
        Hovering { hover in
            Button(action: onShowApp) {
                Text(label).font(.ui(size, weight: .semibold)).foregroundStyle(Theme.bronzo).opacity(hover ? 0.8 : 1)
            }
            .buttonStyle(.plain)
            .help("Open Vorn")
        }
    }
}

/// Widget.tsx `AgentSection`: one session's row; a click brings it up in the window.
private struct WidgetRow: View {
    let agent: WidgetAgent
    let action: () -> Void
    @State private var pulse = false

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 10) {
                    icon
                    Text(agent.label).font(.ui(12, weight: .medium)).foregroundStyle(Theme.white(0.9))
                        .lineLimit(1).truncationMode(.tail)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    StatusDot(status: agent.status, size: 7)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(hover ? Theme.white(0.06) : .clear)
                .animation(.easeOut(duration: 0.15), value: hover)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(agent.tooltip)
        }
    }

    /// widget.css `agent-icon-pulse` and `agent-icon-glow`: a running agent breathes inside a halo of its colour.
    private var icon: some View {
        let running = agent.status == .running
        let glow = WidgetGlow.of(agent.agentType)
        let tint = Color(.sRGB, red: glow.red / 255, green: glow.green / 255, blue: glow.blue / 255, opacity: glow.alpha)
        return ZStack {
            if running {
                // `0 0 8px 3px`: a 3pt ring hugging the 24pt circle, fading out over the next few points.
                Circle().strokeBorder(tint.opacity(0.4), lineWidth: 4).frame(width: 38, height: 38).opacity(pulse ? 1 : 0)
                Circle().strokeBorder(tint, lineWidth: 3).frame(width: 30, height: 30).opacity(pulse ? 1 : 0)
            }
            AgentIcon(agent.agentType, size: 18)
                .scaleEffect(running && pulse ? 1.18 : 1)
                .opacity(running ? (pulse ? 1 : 0.85) : 1)
        }
        .frame(width: 24, height: 24)
        .onAppear { if running { startPulse() } }
        .onChange(of: running) { _, now in
            if now { startPulse() } else { withAnimation(.default) { pulse = false } }
        }
    }

    private func startPulse() {
        pulse = false
        withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { pulse = true }
    }
}

/// widget.css `.status-dot`: running in ink, waiting in the accent, idle receding, errors in danger.
private struct StatusDot: View {
    let status: AgentStatus
    let size: CGFloat

    var body: some View {
        Circle().fill(color).frame(width: size, height: size)
    }

    private var color: Color {
        switch status {
        case .running: Theme.ink
        case .waiting: Theme.bronzo
        case .idle: Theme.inkGhost
        case .error: Theme.danger
        }
    }
}

/// Widget.tsx `PermissionCard` and `AskQuestionCard`: what an agent wants to do, and the answers to give it.
private struct PermissionCardView: View {
    let request: PermissionRequest
    let feed: WidgetFeed
    @State private var picks: [Int: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if request.isQuestion { question } else { permission }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.1), lineWidth: 1))
    }

    private func toolBadge(_ text: String) -> some View {
        Text(text).font(.system(size: 10, weight: .semibold, design: .monospaced)).foregroundStyle(Theme.white(0.45))
            .padding(.horizontal, 6).padding(.vertical, 1)
            .background(Theme.white(0.08), in: RoundedRectangle(cornerRadius: Theme.radius))
            .padding(.bottom, 6)
    }

    @ViewBuilder private var permission: some View {
        toolBadge(request.toolName)
        if let context = request.context {
            HStack(spacing: 5) {
                Text(context.kind == .file ? "◈" : "$").font(.ui(10)).foregroundStyle(Theme.white(0.25))
                Text(context.text).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.white(0.75))
                    .lineLimit(1).truncationMode(.tail)
            }
            .padding(.horizontal, 7)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 5))
            .padding(.bottom, 8)
        }
        HStack(spacing: 6) {
            WidgetActionButton(label: "Allow", kind: .allow) { feed.respond(request.requestId, allow: true) }
            WidgetActionButton(label: "Deny", kind: .deny) { feed.respond(request.requestId, allow: false) }
        }
        if !request.suggestions.isEmpty {
            VStack(spacing: 4) {
                ForEach(Array(request.suggestions.enumerated()), id: \.offset) { _, s in
                    WidgetActionButton(label: PermissionRequest.label(suggestion: s, toolName: request.toolName),
                                       kind: .suggestion) {
                        feed.respond(request.requestId, allow: true, updatedPermissions: [s])
                    }
                }
            }
            .padding(.top, 5)
        }
    }

    @ViewBuilder private var question: some View {
        toolBadge("Question")
        ForEach(Array(request.questions.enumerated()), id: \.offset) { qi, q in
            VStack(alignment: .leading, spacing: 0) {
                if let header = q.header, !header.isEmpty {
                    Text(header.uppercased()).font(.ui(10, weight: .semibold)).tracking(0.4)
                        .foregroundStyle(Theme.white(0.35)).padding(.bottom, 3)
                }
                Text(q.question).font(.ui(11)).foregroundStyle(Theme.white(0.8)).lineSpacing(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.bottom, 6)
                VStack(spacing: 3) {
                    ForEach(q.options, id: \.self) { label in
                        WidgetActionButton(label: label, kind: picks[qi] == label ? .picked : .option) { picks[qi] = label }
                    }
                }
            }
            .padding(.bottom, 8)
        }
        let answered = request.questions.indices.allSatisfy { picks[$0] != nil }
        HStack(spacing: 6) {
            WidgetActionButton(label: "Submit", kind: .allow) {
                feed.respond(request.requestId, allow: true, updatedInput: request.answering(picks))
            }
            .opacity(answered ? 1 : 0.4)
            .disabled(!answered)
            WidgetActionButton(label: "Skip", kind: .deny) { feed.respond(request.requestId, allow: false) }
        }
    }
}

/// widget.css's buttons: Allow, Deny, a suggested rule, and a question's options.
private struct WidgetActionButton: View {
    enum Kind { case allow, deny, suggestion, option, picked }
    let label: String
    let kind: Kind
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                Text(label)
                    .font(.ui(fontSize, weight: weight))
                    .foregroundStyle(text(hover))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: centred ? .center : .leading)
                    .padding(.horizontal, centred ? 0 : 8)
                    .padding(.vertical, kind == .suggestion ? 4 : 5)
                    .background(fill(hover), in: RoundedRectangle(cornerRadius: 5))
                    .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(border(hover), lineWidth: 1))
                    .contentShape(Rectangle())
                    .animation(.easeOut(duration: 0.15), value: hover)
            }
            .buttonStyle(.plain)
        }
    }

    private var centred: Bool { kind == .allow || kind == .deny }
    private var fontSize: CGFloat { kind == .suggestion ? 10 : 11 }
    private var weight: Font.Weight {
        switch kind {
        case .allow, .deny: .semibold
        case .suggestion: .medium
        case .option, .picked: .regular
        }
    }

    private func text(_ hover: Bool) -> Color {
        switch kind {
        case .allow: hover ? Theme.ink : Theme.white(0.8)
        case .deny: hover ? Theme.danger : Theme.white(0.4)
        case .suggestion: Theme.white(hover ? 0.7 : 0.35)
        case .option: Theme.white(hover ? 0.8 : 0.55)
        case .picked: Theme.white(0.9)
        }
    }

    private func fill(_ hover: Bool) -> Color {
        switch kind {
        case .allow: Theme.white(hover ? 0.16 : 0.09)
        case .deny: hover ? Theme.danger.opacity(0.14) : Theme.white(0.04)
        case .suggestion: hover ? Theme.white(0.08) : .clear
        case .option: hover ? Theme.white(0.05) : .clear
        case .picked: Theme.white(0.12)
        }
    }

    private func border(_ hover: Bool) -> Color {
        switch kind {
        case .allow: Theme.white(hover ? 0.28 : 0.14)
        case .deny: hover ? Theme.danger.opacity(0.3) : Theme.white(0.08)
        case .suggestion: Theme.white(hover ? 0.18 : 0.06)
        case .option: Theme.white(0.08)
        case .picked: Theme.white(0.35)
        }
    }
}

/// Widget.tsx's header and pill buttons, drawing the renderer's inline glyphs.
private struct WidgetGlyphButton<G: Shape>: View {
    let size: CGFloat
    let glyph: CGFloat
    let radius: CGFloat
    let color: Double
    let hoverFill: Double
    let help: String
    let action: () -> Void
    @ViewBuilder let shape: () -> G

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                shape()
                    .stroke(style: StrokeStyle(lineWidth: glyph == 12 ? 1.5 : 1.2, lineCap: .round))
                    .frame(width: glyph, height: glyph)
                    .foregroundStyle(Theme.white(hover ? 0.75 : color))
                    .frame(width: size, height: size)
                    .background(hover ? Theme.white(hoverFill) : .clear, in: RoundedRectangle(cornerRadius: radius))
                    .contentShape(Rectangle())
                    .animation(.easeOut(duration: 0.15), value: hover)
            }
            .buttonStyle(.plain)
            .help(help)
        }
    }
}

/// The renderer's inline SVG glyphs, in their own viewBox units.
private enum WidgetGlyph {
    /// `M3 6H9` in a 12 box.
    static var minus: Path {
        Path { p in
            p.move(to: CGPoint(x: 3, y: 6))
            p.addLine(to: CGPoint(x: 9, y: 6))
        }
    }

    /// An x from `inset` to `box - inset`.
    static func close(inset: CGFloat, of box: CGFloat) -> Path {
        Path { p in
            p.move(to: CGPoint(x: inset, y: inset))
            p.addLine(to: CGPoint(x: box - inset, y: box - inset))
            p.move(to: CGPoint(x: box - inset, y: inset))
            p.addLine(to: CGPoint(x: inset, y: box - inset))
        }
    }

    /// A 7-wide window with a title bar, in a 10 box.
    static var expand: Path {
        Path { p in
            p.addRoundedRect(in: CGRect(x: 1.5, y: 1.5, width: 7, height: 7), cornerSize: CGSize(width: 1.2, height: 1.2))
            p.move(to: CGPoint(x: 1.5, y: 3.5))
            p.addLine(to: CGPoint(x: 8.5, y: 3.5))
        }
    }
}

/// index.ts's widget window: a floating panel over every space, shown while the main window is away.
@MainActor
final class WidgetController {
    let feed: WidgetFeed
    private let store: VornStore
    private let model: AppModel
    private var panel: NSPanel?
    /// index.ts `widgetEnabled`: the setting, or turned on for this run by Toggle Widget.
    private(set) var enabled = false
    private var observers: [NSObjectProtocol] = []
    private var watch: Task<Void, Never>?

    init(store: VornStore, model: AppModel) {
        self.store = store
        self.model = model
        feed = WidgetFeed(store: store)
    }

    /// Starts following the setting, the app's focus and the permission requests.
    func install() {
        guard observers.isEmpty else { return }
        feed.start()
        feed.onRequest = { [weak self] in
            if !NSApp.isActive { self?.show() }
        }
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(100))
                guard let self, !NSApp.isActive else { return }
                self.show()
            }
        })
        observers.append(center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.hide() }
        })
        observers.append(center.addObserver(forName: NSWindow.willCloseNotification, object: nil, queue: .main) { [weak self] note in
            let closing = note.object as? NSWindow
            Task { @MainActor in
                guard let self, let closing, closing !== self.panel, closing.canBecomeMain else { return }
                self.show()
            }
        })
        watch = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                let on = self.store.defaults?.isWidgetEnabled ?? false
                if on != self.enabled {
                    self.enabled = on
                    if !on { self.hide() }
                }
                await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
                    withObservationTracking { _ = self.store.defaults } onChange: { done.resume() }
                }
            }
        }
    }

    /// index.ts `toggleWidget`: shows it, turning it on for this run, or hides it.
    func toggle() {
        if panel?.isVisible == true {
            hide()
        } else {
            enabled = true
            show()
        }
    }

    /// index.ts `showWidget`: only while it is turned on, and without taking focus.
    func show() {
        guard enabled else { return }
        let panel = self.panel ?? makePanel()
        if !panel.isVisible { panel.orderFrontRegardless() }
    }

    func hide() {
        panel?.orderOut(nil)
    }

    private func makePanel() -> NSPanel {
        let size = feed.mode.size
        let area = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let frame = NSRect(x: area.maxX - size.width - WidgetPlacement.margin, y: area.minY + WidgetPlacement.margin,
                           width: size.width, height: size.height)
        let panel = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.contentView = NSHostingView(rootView: WidgetPanelRoot(controller: self))
        self.panel = panel
        return panel
    }

    /// index.ts `widget:set-view-mode`: the panel takes the new size, keeping its bottom-right corner on screen.
    func setMode(_ mode: WidgetViewMode) {
        feed.mode = mode
        guard let panel else { return }
        let screen = panel.screen ?? NSScreen.main
        let area = screen?.visibleFrame ?? panel.frame
        // AppKit counts y up from the bottom; WidgetPlacement counts down from the top, as the desktop app does.
        let flip = { (r: CGRect) in CGRect(x: r.minX, y: area.maxY - r.maxY + area.minY, width: r.width, height: r.height) }
        let next = flip(WidgetPlacement.resized(flip(panel.frame), to: mode.size, in: area))
        panel.setFrame(next, display: true)
    }

    /// index.ts `widget:focus-terminal`: brings the window forward on that session.
    func focus(_ id: String) {
        model.setFocused(id)
        showApp()
    }

    /// index.ts `widget:show-app`.
    func showApp() {
        NSApp.activate(ignoringOtherApps: true)
        NSApp.windows.first { $0 !== panel && $0.canBecomeMain }?.makeKeyAndOrderFront(nil)
    }
}

/// The panel's SwiftUI root, reading the controller's feed.
private struct WidgetPanelRoot: View {
    let controller: WidgetController

    var body: some View {
        WidgetView(feed: controller.feed, agents: controller.feed.agents,
                   onFocus: { controller.focus($0) }, onShowApp: { controller.showApp() },
                   onHide: { controller.hide() }, onMode: { controller.setMode($0) })
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}
