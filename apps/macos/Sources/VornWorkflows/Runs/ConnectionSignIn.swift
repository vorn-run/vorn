import AppKit
import CryptoKit
import Foundation
import VornCore
import WebKit

/// How a sign-in ended (connection-sessions.ts SignInResult).
public struct SignInResult: Equatable, Sendable {
    public var ok: Bool
    public var identity: String?
    public var error: String?
}

/// What `connection:browserAuth` answers for a connection that signs in through a window.
struct BrowserSignIn: Equatable {
    var name: String
    var signInURL: URL
    var origins: [String]
    var checkURL: URL
    var checkHeaders: [String: String]
    var identityPaths: [String]

    init?(_ json: JSONValue) {
        let browser = json["browser"]
        guard let name = json["name"]?.stringValue,
              let signIn = browser?["signInUrl"]?.stringValue.flatMap(URL.init(string:)),
              let check = browser?["check"]?["url"]?.stringValue.flatMap(URL.init(string:)) else { return nil }
        self.name = name
        signInURL = signIn
        checkURL = check
        origins = browser?["origins"]?.arrayValue?.compactMap(\.stringValue) ?? []
        var headers: [String: String] = [:]
        for (key, value) in browser?["check"]?["headers"]?.objectValue ?? [:] {
            if let value = value.stringValue, Self.allowedHeader(key) { headers[key] = value }
        }
        checkHeaders = headers
        identityPaths = browser?["check"]?["identity"]?.arrayValue?.compactMap(\.stringValue) ?? []
    }

    /// The headers the check asks for, with JSON accepted unless it says otherwise.
    var requestHeaders: [String: String] {
        checkHeaders.keys.contains { $0.lowercased() == "accept" }
            ? checkHeaders : checkHeaders.merging(["accept": "application/json"]) { a, _ in a }
    }

    /// connector-origins.ts allowedSessionHeader: the browser sets the rest, or they carry who you are.
    static func allowedHeader(_ name: String) -> Bool {
        let lower = name.lowercased()
        let forbidden: Set = ["cookie", "cookie2", "authorization", "host", "origin", "referer", "content-length"]
        return name.wholeMatch(of: #/[A-Za-z0-9!#$%&'*+.^_`|~-]+/#) != nil
            && !forbidden.contains(lower) && !lower.hasPrefix("sec-") && !lower.hasPrefix("proxy-")
    }

    /// connector-origins.ts withinOrigins: https on the default port, on a declared host or under a wildcard.
    static func within(_ origins: [String], _ url: URL) -> Bool {
        guard url.scheme == "https", url.port == nil, let host = url.host?.lowercased() else { return false }
        return origins.contains { origin in
            guard origin.wholeMatch(of: #/https:\/\/(\*\.)?[a-zA-Z0-9-]+(\.[a-zA-Z0-9-]+)+/#) != nil else { return false }
            let wildcard = origin.hasPrefix("https://*.")
            let declared = origin.dropFirst(wildcard ? 10 : 8).lowercased()
            return wildcard ? host.hasSuffix(".\(declared)") : host == declared
        }
    }

    /// connection-session-script.ts identityFrom: the first named value, the rest in parentheses.
    static func identity(_ body: String, paths: [String]) -> String? {
        guard let parsed = try? JSONSerialization.jsonObject(with: Data(body.utf8), options: [.fragmentsAllowed])
        else { return nil }
        let values: [String] = paths.compactMap { path in
            let value = path.split(separator: ".", omittingEmptySubsequences: false).reduce(Optional(parsed)) { at, key in
                (at as? [String: Any])?[String(key)]
            }
            if let s = value as? String, !s.trimmingCharacters(in: .whitespaces).isEmpty { return s }
            if let n = value as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() { return n.stringValue }
            return nil
        }
        guard let first = values.first else { return nil }
        return values.count > 1 ? "\(first) (\(values.dropFirst().joined(separator: ", ")))" : first
    }
}

/// A connection's sign-in window, in a profile of its own that keeps the session it signs into;
/// it closes once the connector's check says someone is signed in (connection-sessions.ts).
@MainActor
public final class ConnectionSignIn: NSObject {
    public static let shared = ConnectionSignIn()

    static let checkEvery: Duration = .seconds(2)
    static let timeout: Duration = .seconds(10 * 60)

    private var signing: [String: Task<SignInResult, Never>] = [:]
    private var windows: [String: NSWindow] = [:]
    private var closed: [String: CheckedContinuation<Void, Never>] = [:]
    private var children: [NSWindow] = []

    public func signIn(_ connectionId: String, backend: any WorkflowsBackend) async -> SignInResult {
        if let pending = signing[connectionId] {
            windows[connectionId]?.makeKeyAndOrderFront(nil)
            return await pending.value
        }
        let run = Task { await open(connectionId, backend: backend) }
        signing[connectionId] = run
        let result = await run.value
        signing[connectionId] = nil
        return result
    }

    /// The connection's own store, the same on every launch, so a sign-in outlives the app.
    static func dataStore(_ connectionId: String) -> WKWebsiteDataStore {
        var bytes = Array(SHA256.hash(data: Data("vorn-connection:\(connectionId)".utf8)).prefix(16))
        bytes[6] = (bytes[6] & 0x0F) | 0x50
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        let id = UUID(uuid: (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                             bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]))
        return WKWebsiteDataStore(forIdentifier: id)
    }

    private func configuration(_ connectionId: String) -> WKWebViewConfiguration {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = Self.dataStore(connectionId)
        return config
    }

    private func open(_ connectionId: String, backend: any WorkflowsBackend) async -> SignInResult {
        guard let raw = try? await backend.rpc("connection:browserAuth", .string(connectionId)),
              let target = BrowserSignIn(raw) else {
            return SignInResult(ok: false, error: "This connection does not sign in through a Vorn window.")
        }
        guard BrowserSignIn.within(target.origins, target.checkURL) else {
            return SignInResult(ok: false, error: "\(target.checkURL.absoluteString) is not on one of this connection's origins")
        }
        let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 520, height: 720), configuration: configuration(connectionId))
        web.uiDelegate = self
        web.navigationDelegate = self
        let window = NSWindow(
            contentRect: web.frame, styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered, defer: false)
        window.title = "Sign in · \(target.name)"
        window.isReleasedWhenClosed = false
        window.contentView = web
        window.delegate = self
        window.center()
        windows[connectionId] = window
        web.load(URLRequest(url: target.signInURL))
        window.makeKeyAndOrderFront(nil)

        let checker = SessionCheck(connectionId: connectionId, target: target, configuration: configuration(connectionId))
        let result = await withTaskGroup(of: SignInResult.self) { group in
            group.addTask { @MainActor in
                while !Task.isCancelled {
                    try? await Task.sleep(for: Self.checkEvery)
                    guard !Task.isCancelled, let identity = await checker.signedIn() else { continue }
                    _ = try? await backend.rpc("connection:signedIn", .object([
                        "connectionId": .string(connectionId), "identity": identity.map(JSONValue.string) ?? .null,
                    ]))
                    return SignInResult(ok: true, identity: identity)
                }
                return SignInResult(ok: false)
            }
            group.addTask {
                try? await Task.sleep(for: Self.timeout)
                return SignInResult(ok: false, error: "Sign-in timed out. Try again.")
            }
            group.addTask { @MainActor in
                await withCheckedContinuation { self.closed[connectionId] = $0 }
                return SignInResult(ok: false, error: "The sign-in window closed before anyone signed in.")
            }
            let first = await group.next() ?? SignInResult(ok: false)
            group.cancelAll()
            closed.removeValue(forKey: connectionId)?.resume()
            return first
        }
        checker.close()
        windows[connectionId] = nil
        window.delegate = nil
        window.close()
        for child in children where child.parent == window || child.parent == nil { child.close() }
        children.removeAll { !$0.isVisible }
        return result
    }
}

extension ConnectionSignIn: NSWindowDelegate {
    public nonisolated func windowWillClose(_ notification: Notification) {
        MainActor.assumeIsolated {
            guard let window = notification.object as? NSWindow,
                  let id = windows.first(where: { $0.value === window })?.key else { return }
            closed.removeValue(forKey: id)?.resume()
        }
    }
}

extension ConnectionSignIn: WKUIDelegate, WKNavigationDelegate {
    /// Providers open windows of their own; they share the profile so the session lands in it.
    public func webView(
        _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction, windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        guard action.request.url?.scheme == "https" else { return nil }
        let child = WKWebView(frame: CGRect(x: 0, y: 0, width: 480, height: 640), configuration: configuration)
        child.uiDelegate = self
        child.navigationDelegate = self
        let window = NSWindow(
            contentRect: child.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = child
        webView.window?.addChildWindow(window, ordered: .above)
        window.center()
        window.makeKeyAndOrderFront(nil)
        children.append(window)
        return child
    }

    public func webViewDidClose(_ webView: WKWebView) {
        guard let window = webView.window, children.contains(window) else { return }
        window.close()
        children.removeAll { $0 === window }
    }

    /// No downloads, and no camera or microphone.
    public func webView(
        _ webView: WKWebView, decidePolicyFor response: WKNavigationResponse,
        decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void
    ) {
        decisionHandler(response.canShowMIMEType ? .allow : .cancel)
    }

    public func webView(
        _ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void
    ) {
        decisionHandler(.deny)
    }
}

/// A hidden page on the check's origin that asks, with the profile's cookies, whether anyone is signed in.
@MainActor
final class SessionCheck {
    let target: BrowserSignIn
    private let web: WKWebView
    private var ready = false

    init(connectionId: String, target: BrowserSignIn, configuration: WKWebViewConfiguration) {
        self.target = target
        web = WKWebView(frame: .zero, configuration: configuration)
    }

    /// nil while no one is signed in; otherwise who, which may itself be unknown.
    func signedIn() async -> String?? {
        guard let origin = originURL else { return nil }
        if !ready || web.url?.host != origin.host {
            // A small same-origin page to stand on, so the check is one the site's own page could make.
            web.load(URLRequest(url: origin.appending(path: "robots.txt")))
            var waited = 0
            while (web.isLoading || web.url == nil) && waited < 50 {
                try? await Task.sleep(for: .milliseconds(100))
                waited += 1
            }
            guard web.url?.host == origin.host else { return nil }
            ready = true
        }
        let script = """
            const res = await fetch(url, { method: 'GET', headers, credentials: 'include' })
            return { status: res.status, body: (await res.text()).slice(0, 4194304) }
            """
        let args: [String: Any] = ["url": target.checkURL.absoluteString, "headers": target.requestHeaders]
        guard let answer = try? await web.callAsyncJavaScript(script, arguments: args, contentWorld: .defaultClient)
                as? [String: Any],
              let status = (answer["status"] as? NSNumber)?.intValue, (200..<300).contains(status) else { return nil }
        return .some(BrowserSignIn.identity(answer["body"] as? String ?? "", paths: target.identityPaths))
    }

    private var originURL: URL? {
        guard let host = target.checkURL.host else { return nil }
        return URL(string: "https://\(host)")
    }

    func close() {
        web.stopLoading()
    }
}
