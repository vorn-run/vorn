import AppKit
import SwiftUI
import VornCore
import VornUI
import WebKit

/// GateReviewModal.tsx: a gate's review page in a sandboxed frame, with the
/// gate's answers under it; Escape or the X closes it.
struct GateReviewModal: View {
    let store: WorkflowsStore
    let target: GateReviewTarget

    @State private var url: URL?
    @State private var failed = false

    var body: some View {
        let run = store.findRun(target.runId)
        let nodes = run.flatMap { store.ref(for: $0)?.nodes } ?? []
        let node = nodes.first { $0.id == target.nodeId }
        let state = run?.nodeStates.first { $0.nodeId == target.nodeId }
        GeometryReader { geo in
            RunModal(backdrop: 0.6) {
                VStack(spacing: 0) {
                    header(node: node, state: state)
                    Hairline(opacity: 0.08)
                    page(state: state, label: node?.label ?? "")
                    Hairline(opacity: 0.08)
                    footer(run: run, node: node, state: state, nodes: nodes, width: min(geo.size.width * 0.9, 960))
                }
                .frame(width: min(geo.size.width * 0.9, 960), height: geo.size.height * 0.84)
                .background(Theme.surfacePanel)
                .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.1), lineWidth: 1))
            }
        }
        .background { Button("", action: close).keyboardShortcut(.cancelAction).opacity(0) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Review: \(node?.label ?? "")")
        .task(id: state?.viewToken) { await resolve(state?.viewToken) }
        // Like today's, the modal goes once the gate is answered.
        .onChange(of: state?.status == .waiting) { _, waiting in if !waiting { close() } }
    }

    private func close() { store.surfaces.review = nil }

    private func resolve(_ token: String?) async {
        url = nil
        failed = false
        guard let token else { return }
        if let next = await store.gateViewURL(runId: target.runId, nodeId: target.nodeId, token: token) {
            url = next
        } else {
            failed = true
        }
    }

    private func header(node: WorkflowNode?, state: NodeExecutionState?) -> some View {
        let meta = [
            target.workflowName, "run \(target.runId.prefix(8))",
            GateRounds.label(node?.config, round: state?.round),
        ].compactMap { $0 }.joined(separator: " · ")
        return HStack(spacing: 10) {
            if let node {
                NodeTypeIcon(type: node.type, size: 13, strokeWidth: 1.5).foregroundStyle(Theme.inkFaint)
            }
            Text(node?.label ?? "").font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.ink)
                .fixedSize()
            Text(meta).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(Theme.inkFaint)
                .lineLimit(1).truncationMode(.tail)
            Spacer(minLength: 0)
            Hovering { hovered in
                Button(action: close) {
                    LucideIcon(.x, size: 14).foregroundStyle(hovered ? Theme.ink : Theme.inkFaint).padding(4)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close review")
            }
        }
        .padding(.horizontal, 16)
        .frame(height: 44)
    }

    @ViewBuilder
    private func page(state: NodeExecutionState?, label: String) -> some View {
        if let url {
            GateReviewPage(url: url, label: label, onFail: { failed = true; self.url = nil })
                .id(url)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.white)
        } else {
            Text(failed || state?.viewToken == nil ? "The review page could not be opened." : "Opening the review page…")
                .font(.system(size: 12))
                .foregroundStyle(Theme.inkFaint)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private func footer(
        run: WorkflowExecution?, node: WorkflowNode?, state: NodeExecutionState?, nodes: [WorkflowNode], width: CGFloat
    ) -> some View {
        HStack(spacing: 16) {
            HStack(spacing: 6) {
                LucideIcon(.lock, size: 11, strokeWidth: 2)
                builtBy(node: node, nodes: nodes)
                    .lineLimit(1).truncationMode(.tail)
            }
            .font(.system(size: 11))
            .foregroundStyle(Theme.inkFaint)
            .frame(maxWidth: .infinity, alignment: .leading)
            if let run, let state {
                GateActions(store: store, run: run, state: state, config: node?.config, nodes: nodes)
                    .frame(width: min(440, (width - 32) * 0.55))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private func builtBy(node: WorkflowNode?, nodes: [WorkflowNode]) -> Text {
        let sandboxed = Text("It runs sandboxed: nothing loads from the network and it can't reach Vorn.")
        guard let producer = Self.producer(view: node?.config["view"]?.stringValue, nodes: nodes) else { return sandboxed }
        return Text("Built by ") + Text(producer).foregroundColor(Theme.inkSecondary) + Text(". ") + sandboxed
    }

    /// The step whose output the page is, so the footer can say what built it.
    static func producer(view: String?, nodes: [WorkflowNode]) -> String? {
        guard let view, let match = view.firstMatch(of: #/\{\{\s*steps\.([\w-]+)\./#) else { return nil }
        let slug = String(match.1)
        return nodes.first { $0.slug == slug }?.label
    }
}

/// The page in a frame of its own: scripts run, nothing persists, and it cannot navigate away.
struct GateReviewPage: NSViewRepresentable {
    let url: URL
    let label: String
    let onFail: () -> Void
    @Environment(\.workflowsStaticLayout) private var snapshot

    func makeCoordinator() -> Coordinator { Coordinator(url: url, onFail: onFail) }

    func makeNSView(context: Context) -> NSView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        config.preferences.javaScriptCanOpenWindowsAutomatically = false
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.setAccessibilityLabel("Review page for \(label)")
        web.underPageBackgroundColor = .white
        web.load(URLRequest(url: url))
        guard snapshot else { return web }
        // Offscreen renders cannot capture the web content process, so they draw a snapshot of it.
        let host = SnapshotHost(web: web)
        context.coordinator.snapshotHost = host
        return host
    }

    func updateNSView(_ view: NSView, context: Context) {}

    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        let web = (view as? WKWebView) ?? (view as? SnapshotHost)?.web
        web?.stopLoading()
        web?.navigationDelegate = nil
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        let url: URL
        let onFail: () -> Void
        weak var snapshotHost: SnapshotHost?

        init(url: URL, onFail: @escaping () -> Void) {
            self.url = url
            self.onFail = onFail
        }

        func webView(
            _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
            decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
        ) {
            // Only the page itself loads in the frame; links and redirects go nowhere.
            let isPage = action.request.url == url && action.targetFrame?.isMainFrame == true
            decisionHandler(isPage || action.targetFrame?.isMainFrame == false ? .allow : .cancel)
        }

        func webView(
            _ webView: WKWebView, decidePolicyFor response: WKNavigationResponse,
            decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void
        ) {
            let ok = (response.response as? HTTPURLResponse).map { (200..<300).contains($0.statusCode) } ?? true
            decisionHandler(ok || !response.isForMainFrame ? .allow : .cancel)
            if !ok && response.isForMainFrame { onFail() }
        }

        func webView(_ webView: WKWebView, didFailProvisionalNavigation _: WKNavigation!, withError error: Error) {
            if (error as NSError).code != NSURLErrorCancelled { onFail() }
        }

        func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
            snapshotHost?.capture()
        }
    }

    /// Holds the web view off to the side and shows a snapshot of it in its place.
    final class SnapshotHost: NSImageView {
        let web: WKWebView
        private let stage = NSWindow(contentRect: .zero, styleMask: .borderless, backing: .buffered, defer: false)

        init(web: WKWebView) {
            self.web = web
            super.init(frame: .zero)
            imageScaling = .scaleAxesIndependently
            stage.contentView = web
        }

        @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

        override func setFrameSize(_ size: NSSize) {
            super.setFrameSize(size)
            stage.setContentSize(size)
            web.frame = CGRect(origin: .zero, size: size)
        }

        func capture() {
            web.takeSnapshot(with: nil) { [weak self] image, _ in self?.image = image }
        }
    }
}
