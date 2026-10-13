import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// BrowserCard.tsx: a session's browser pane, its artifacts and designs.
public struct BrowserCard: View {
    @Bindable var panes: PanesModel
    @Bindable var model: BrowserModel
    let sessionID: String
    let services: PaneServices

    public init(panes: PanesModel, sessionID: String, services: PaneServices) {
        self.panes = panes
        self.model = panes.browser
        self.sessionID = sessionID
        self.services = services
    }

    public var body: some View {
        if let session = services.session(sessionID), let pane = model.panes[sessionID], let tab = pane.active {
            let s = sessionID
            let l = model.liveState(s)
            let art = model.art(s)
            let st = model.artState(s)
            let boards = model.boards(s)
            let editing = art != nil ? l.editing : nil
            let agent = session.agentType.rawValue
            VStack(spacing: 0) {
                PaneTabStrip(tabs: tabs(pane, manifest: l.manifest), activeID: String(pane.activeTab),
                             onSelect: { model.selectTab(s, Int($0) ?? 0) },
                             onClose: { model.closeTab(s, Int($0) ?? 0) },
                             onAdd: { model.addTab(s) }) {
                    PaneControls(panes: panes, paneID: PaneID.browser(s), title: BrowserURL.displayHost(tab.current),
                                 onClose: { model.close(s) })
                }
                HStack(spacing: 2) {
                    if let art, editing != nil {
                        DocEditBar(version: art.version) { model.discard(s) }
                    } else if let art {
                        ArtifactBar(model: model, s: s, art: art, st: st, agent: agent, commenting: l.commenting,
                                    sending: l.sending, versionsOpen: l.versions)
                        if let m = l.manifest, !m.tweaks.isEmpty, boards == nil {
                            TweakBar(manifest: m, values: l.values) { model.applyTweak(s, key: $0, value: $1) }
                        }
                    } else if let m = l.manifest, m.kind == "design" {
                        if boards == nil { TweakBar(manifest: m, values: l.values) { model.applyTweak(s, key: $0, value: $1) } }
                        Spacer(minLength: 0)
                    } else {
                        AddressBar(model: model, s: s, url: tab.current, live: l)
                    }
                    if editing == nil && boards == nil {
                        PaneIconButton(icon: .mousePointerClick, help: "Point at an element to describe it to this session's agent",
                                       active: l.picking) { model.pick(s) }
                            .accessibilityLabel("Pick an element for the agent")
                            .padding(.leading, 4)
                        PaneIconButton(icon: .pencil, help: "Draw over the page, then click again to send it to this session's agent") {
                            model.ink(s)
                        }
                        .accessibilityLabel("Draw on the page for the agent")
                    }
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 4)
                if let art, let st, l.dismissed != "\(art.id):\(art.version)" {
                    ArtifactBanner(version: art.version, st: st, comparing: l.comparing,
                                   onCompare: { model.toggleCompare(s) }, onOpenLatest: { model.openLatest(s) },
                                   onDismiss: { model.dismissBanner(s) })
                }
                if let failed = l.failed {
                    Text(failed).font(.ui(10)).foregroundStyle(Theme.amber400.opacity(0.9))
                        .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 8).padding(.vertical, 4)
                }
                content(s, tab: tab, l: l, art: art, st: st, boards: boards, agent: agent)
                    .background(Theme.surfacePanel)
            }
            .background(Theme.surfacePanel)
            .onExitCommand { model.closeMenus(s) }
        }
    }

    @ViewBuilder
    private func content(_ s: String, tab: BrowserTab, l: BrowserLive, art: TabArtifact?, st: ArtifactState?,
                         boards: [Artboard]?, agent: String) -> some View {
        let comparing = l.comparing && l.compareUrl != nil
        let editing = art != nil ? l.editing : nil
        HStack(spacing: 0) {
            if comparing, let on = art.flatMap({ st?.answered($0.version).on }) {
                VStack(spacing: 0) {
                    versionHeader(on)
                    WebSlot(model: model, key: "\(s)#compare", token: l.compareUrl ?? "")
                }
                .frame(maxWidth: .infinity)
                Rectangle().fill(Theme.hairline).frame(width: 1)
            }
            VStack(spacing: 0) {
                if comparing, let art { versionHeader(art.version) }
                GeometryReader { geo in
                    ZStack {
                        WebSlot(model: model, key: s, token: tab.url)
                        if let editing, let art {
                            DocEditor(draft: editing, drafts: model.comments(s).drafts.count,
                                      next: (st?.artifact.latestVersion ?? art.version) + 1, agent: agent, saving: l.saving,
                                      onChange: { model.setEdited(s, $0) }, onSave: { model.saveDoc(s, send: $0) })
                        }
                        if let boards, editing == nil, let selected = model.board(s) {
                            DesignCanvasView(model: model, s: s, boards: boards, selected: selected.id, area: geo.size)
                        }
                    }
                    .onAppear { model.areas[s] = geo.size }
                    .onChange(of: geo.size) { _, size in model.areas[s] = size }
                }
            }
            .frame(maxWidth: .infinity)
            if boards != nil, editing == nil, let selected = model.board(s), let m = l.manifest, !m.tweaks.isEmpty {
                TweaksAside(label: selected.label, manifest: m, values: l.values) { model.applyTweak(s, key: $0, value: $1) }
            }
            if l.commenting, let art, editing == nil {
                ArtifactRail(model: model, s: s, version: art.version, agent: agent, queued: st?.queued ?? false,
                             sending: l.sending, found: l.found, railEdit: l.railEdit,
                             hint: boards != nil ? "Click an artboard to pin a comment there." : nil)
            }
        }
    }

    private func versionHeader(_ v: Int) -> some View {
        Text("v\(v)").font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.inkFaint)
            .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 10).frame(height: 24)
    }

    private func tabs(_ pane: BrowserPane, manifest: Manifest?) -> [PaneTab] {
        pane.tabs.enumerated().map { i, tab in
            let shown = tab.current
            let host = BrowserURL.displayHost(shown)
            let active = i == pane.activeTab
            let label = tab.artifact?.title ?? (active ? manifest?.title : nil) ?? host
            var icon: AnyView?
            if let a = tab.artifact {
                icon = AnyView(LucideIcon(Self.kindIcon(a.kind), size: 11, strokeWidth: 2).foregroundStyle(Theme.ink))
            } else if active, manifest?.kind == "design" {
                icon = AnyView(LucideIcon(.shapes, size: 11, strokeWidth: 2).foregroundStyle(Theme.bronzo))
            }
            return PaneTab(id: String(i), name: label, title: shown, icon: icon)
        }
    }

    static func kindIcon(_ kind: String) -> LucideGlyph {
        switch kind {
        case "doc": .alignLeft
        case "design": .shapes
        default: .fileText
        }
    }
}

/// AddressBar.tsx: history, reload, and somewhere to type a url.
struct AddressBar: View {
    let model: BrowserModel
    let s: String
    let url: String
    let live: BrowserLive
    @FocusState private var focused: Bool

    var body: some View {
        let text = Binding<String>(get: { live.draft ?? (url == BrowserURL.blank ? "" : url) },
                                   set: { model.setDraft(s, $0) })
        PaneIconButton(icon: .arrowLeft, help: "Go back", disabled: !live.nav.back, dimsWhenDisabled: false) { model.back(s) }
            .opacity(live.nav.back ? 1 : 0.25)
        PaneIconButton(icon: .arrowRight, help: "Go forward", disabled: !live.nav.forward, dimsWhenDisabled: false) { model.forward(s) }
            .opacity(live.nav.forward ? 1 : 0.25)
        PaneIconButton(icon: live.loading ? .x : .rotateCw, help: live.loading ? "Stop loading" : "Reload") { model.reloadOrStop(s) }
        Hovering { hover in
            TextField("", text: text, prompt: Text("Type a URL").foregroundColor(Theme.gray500))
                .textFieldStyle(.plain)
                .font(focused ? .system(size: 11, design: .monospaced) : .ui(11))
                .multilineTextAlignment(focused ? .leading : .center)
                .foregroundStyle(Theme.gray300)
                .focused($focused)
                .padding(.horizontal, 12)
                .padding(.vertical, 4)
                .background(Capsule().fill(Theme.white(focused ? 0.07 : hover ? 0.06 : 0.04)))
                .onSubmit { model.submitAddress(s, text.wrappedValue) }
                .onExitCommand { model.setDraft(s, nil) }
                .accessibilityLabel("Address")
        }
        .padding(.leading, 4)
        .frame(maxWidth: .infinity)
    }
}

/// When a version was made, as the bar and the menu say it.
enum VersionTime {
    private static let iso: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    static func ago(_ s: String) -> String {
        guard let d = iso.date(from: s) ?? ISO8601DateFormatter().date(from: s) else { return "" }
        return formatRelativeTime(d.timeIntervalSince1970 * 1000).lowercased()
    }

    static func byline(_ v: ArtifactVersion?, agent: String) -> String {
        guard let v else { return "" }
        return "\(v.author == "user" ? "you" : agent) \u{b7} \(ago(v.createdAt))"
    }
}

/// ArtifactBar.tsx: which version, who made it, and the way back to the agent.
struct ArtifactBar: View {
    let model: BrowserModel
    let s: String
    let art: TabArtifact
    let st: ArtifactState?
    let agent: String
    let commenting: Bool
    let sending: Bool
    let versionsOpen: Bool

    var body: some View {
        let versions = st?.versions ?? []
        let comments = st?.comments ?? []
        let drafts = comments.filter(\.isDraft).count
        let queued = st?.queued ?? false
        let onLatest = art.version == (st?.artifact.latestVersion ?? art.version)
        Hovering { hover in
            Button { model.toggleVersions(s) } label: {
                HStack(spacing: 4) {
                    Text("v\(art.version)")
                    LucideIcon(.chevronDown, size: 11, strokeWidth: 2)
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Theme.inkSecondary)
                .padding(.horizontal, 8)
                .frame(height: 24)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Version \(art.version), choose another")
        }
        .popover(isPresented: Binding(get: { versionsOpen }, set: { if !$0 { model.closeMenus(s) } }), arrowEdge: .bottom) {
            menu(versions, comments)
        }
        Text(VersionTime.byline(st?.version(art.version), agent: agent))
            .font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.inkFaint).lineLimit(1)
            .padding(.leading, 6)
        Spacer(minLength: 0)
        if art.kind == "doc" {
            BarButton(label: "Edit", disabled: !onLatest) { model.edit(s) }
                .help(onLatest ? "Edit the words yourself" : "Open the latest version to edit it")
                .padding(.trailing, 4)
        }
        PaneIconButton(icon: .messageSquarePlus, help: commenting ? "Stop commenting" : "Comment on the page", active: commenting) {
            model.toggleComments(s)
        }
        BarButton(label: queued ? "Queued" : "Send to agent", count: drafts, disabled: drafts == 0 || sending) { model.send(s) }
            .help(queued ? "Queued: goes when \(agent) is at its prompt" : drafts > 0 ? "Send \(drafts) to \(agent)" : "Comment on the page first")
            .padding(.leading, 4)
    }

    private func menu(_ versions: [ArtifactVersion], _ comments: [ArtifactComment]) -> some View {
        VStack(spacing: 0) {
            ForEach(versions.sorted { $0.version > $1.version }, id: \.version) { v in
                let count = comments.filter { $0.version == v.version }.count
                let current = v.version == art.version
                Hovering { hover in
                    Button { model.chooseVersion(s, v.version) } label: {
                        HStack(spacing: 8) {
                            Text("v\(v.version) \u{b7} \(v.author == "user" ? "you" : agent)"
                                + (count > 0 ? " \u{b7} \(count) comment\(count == 1 ? "" : "s")" : ""))
                                .font(.ui(12)).lineLimit(1)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Text(VersionTime.ago(v.createdAt)).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.inkFaint)
                        }
                        .foregroundStyle(current ? Theme.ink : Theme.inkSecondary)
                        .padding(.horizontal, 8)
                        .frame(height: 26)
                        .background(RoundedRectangle(cornerRadius: Theme.radius)
                            .fill(current ? Theme.white(0.08) : hover ? Theme.white(0.05) : .clear))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(4)
        .frame(width: 240)
        .background(Theme.surfaceOverlay)
    }
}

/// The bar's bordered text buttons: Edit, Send to agent.
struct BarButton: View {
    let label: String
    var count: Int?
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    Text(label).font(.ui(12))
                    if let count {
                        Text("\(count)").font(.system(size: 10, weight: .semibold, design: .monospaced))
                            .padding(.horizontal, 4).frame(minWidth: 16, minHeight: 16)
                            .background(Capsule().fill(Theme.white(0.12)))
                    }
                }
                .foregroundStyle(Theme.ink)
                .padding(.horizontal, 10)
                .frame(height: 24)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover && !disabled ? Theme.white(0.06) : .clear))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.12)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.4 : 1)
        }
        .fixedSize()
    }
}

/// ArtifactBanner.tsx: a newer version, or the answer to comments you sent.
struct ArtifactBanner: View {
    let version: Int
    let st: ArtifactState
    let comparing: Bool
    let onCompare: () -> Void
    let onOpenLatest: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        let latest = st.artifact.latestVersion
        let behind = latest > version
        let answered = st.answered(version)
        let n = answered.comments.count
        if behind || n > 0 {
            HStack(spacing: 8) {
                Text(behind ? "v\(latest) is out; you are reading v\(version)."
                    : "v\(version) answers your \(n) comment\(n == 1 ? "" : "s")\(answered.on.map { " on v\($0)" } ?? "").")
                    .lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 0)
                if behind {
                    TextAction(label: "Open v\(latest)", action: onOpenLatest)
                } else if let on = answered.on {
                    TextAction(label: comparing ? "Close compare" : "Compare with v\(on)", pressed: comparing, action: onCompare)
                }
                PaneIconButton(icon: .x, help: "Dismiss", size: 12, action: onDismiss)
            }
            .font(.ui(12))
            .foregroundStyle(Theme.inkSecondary)
            .padding(.horizontal, 10)
            .frame(height: 32)
            .background(Theme.surfaceSunken)
            .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
        }
    }
}

/// A borderless text button that lights on hover.
struct TextAction: View {
    let label: String
    var pressed = false
    var color: Color = Theme.ink
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                Text(label).font(.ui(12)).foregroundStyle(color)
                    .padding(.horizontal, 8).frame(height: 24)
                    .background(RoundedRectangle(cornerRadius: Theme.radius)
                        .fill(pressed ? Theme.white(0.08) : hover ? Theme.white(0.06) : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .fixedSize()
    }
}
