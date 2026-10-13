import AppKit
import CryptoKit
import Foundation
import VornCore
import WebKit

/// What a page view reports.
enum WebEvent {
    case page(key: String, url: String, finished: Bool)
    case title(key: String, title: String)
    case message(key: String, body: String)
    case newWindow(key: String, url: String)
    case failed(key: String, message: String)
}

/// One page view: the address last asked for and how many loads it has seen.
@MainActor
final class WebPage: NSObject, WKNavigationDelegate, WKUIDelegate {
    let key: String
    let view: WKWebView
    var asked: String
    /// Bumped on each load, so a ref from the last page misses.
    var generation = 1
    weak var host: WebHost?
    private var observers: [NSKeyValueObservation] = []

    init(key: String, view: WKWebView, asked: String) {
        self.key = key
        self.view = view
        self.asked = asked
        super.init()
        view.navigationDelegate = self
        view.uiDelegate = self
        observers.append(view.observe(\.title, options: [.new]) { [weak self] v, _ in
            MainActor.assumeIsolated { self?.emit(.title(key: key, title: v.title ?? "")) }
        })
        observers.append(view.observe(\.canGoBack) { [weak self] _, _ in MainActor.assumeIsolated { self?.host?.navChanged(key) } })
        observers.append(view.observe(\.canGoForward) { [weak self] _, _ in MainActor.assumeIsolated { self?.host?.navChanged(key) } })
    }

    private func emit(_ e: WebEvent) { host?.onEvent(e) }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        generation += 1
        emit(.page(key: key, url: webView.url?.absoluteString ?? asked, finished: false))
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        emit(.page(key: key, url: webView.url?.absoluteString ?? asked, finished: true))
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        let e = error as NSError
        guard e.code != NSURLErrorCancelled else { return }
        emit(.failed(key: key, message: e.localizedDescription))
        emit(.page(key: key, url: webView.url?.absoluteString ?? asked, finished: true))
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = navigationAction.request.url?.absoluteString { emit(.newWindow(key: key, url: url)) }
        return nil
    }
}

/// Forwards `window.ipc` posts without the content controller keeping the host alive.
private final class MessageProxy: NSObject, WKScriptMessageHandler {
    weak var host: WebHost?
    let key: String
    init(host: WebHost, key: String) {
        self.host = host
        self.key = key
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? String else { return }
        let key = key
        MainActor.assumeIsolated { host?.onEvent(.message(key: key, body: body)) }
    }
}

/// The web views of every open pane, keyed `session`, `session#compare` and `session#board:id`.
@MainActor
final class WebHost {
    static let pageScript: String = {
        guard let url = Bundle.module.url(forResource: "page", withExtension: "js"),
              let js = try? String(contentsOf: url, encoding: .utf8) else { return "" }
        return js
    }()

    private static let shim = "window.ipc={postMessage:function(s){window.webkit.messageHandlers.vorn.postMessage(s)}};"

    private(set) var pages: [String: WebPage] = [:]
    private var stores: [String: WKWebsiteDataStore] = [:]
    /// Pages keep nothing on disk: tests and renders.
    var ephemeral = false
    var onEvent: (WebEvent) -> Void = { _ in }
    var onNav: (String) -> Void = { _ in }

    func navChanged(_ key: String) { onNav(key) }

    static func sessionOf(_ key: String) -> String { key.split(separator: "#", maxSplits: 1).first.map(String.init) ?? key }

    /// Each session browses in its own store, so logins in one pane do not leak into another's.
    private func store(_ session: String) -> WKWebsiteDataStore {
        if let s = stores[session] { return s }
        let s: WKWebsiteDataStore
        if ephemeral {
            s = .nonPersistent()
        } else {
            let digest = Array(Insecure.MD5.hash(data: Data("vorn-browser:\(session)".utf8)))
            let uuid = UUID(uuid: (digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
                                   digest[8], digest[9], digest[10], digest[11], digest[12], digest[13], digest[14], digest[15]))
            s = WKWebsiteDataStore(forIdentifier: uuid)
        }
        stores[session] = s
        return s
    }

    /// The view for `key`, created and loaded as needed; a new `url` loads.
    @discardableResult
    func page(_ key: String, url: String, root: String?) -> WebPage {
        if let p = pages[key] {
            if p.asked != url {
                p.asked = url
                Self.load(p.view, url, root: root)
            }
            return p
        }
        let config = WKWebViewConfiguration()
        config.websiteDataStore = store(Self.sessionOf(key))
        let content = WKUserContentController()
        content.addUserScript(WKUserScript(source: Self.shim + Self.pageScript, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController = content
        let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        view.allowsBackForwardNavigationGestures = true
        let page = WebPage(key: key, view: view, asked: url)
        page.host = self
        content.add(MessageProxy(host: self, key: key), name: "vorn")
        pages[key] = page
        Self.load(view, url, root: root)
        return page
    }

    private static func load(_ view: WKWebView, _ url: String, root: String?) {
        guard let u = URL(string: url) else { return }
        if u.isFileURL {
            view.loadFileURL(u, allowingReadAccessTo: URL(fileURLWithPath: root ?? u.deletingLastPathComponent().path))
        } else {
            view.load(URLRequest(url: u))
        }
    }

    /// Drops the views whose key `gone` says goes.
    func close(where gone: (String) -> Bool) {
        for (k, p) in pages where gone(k) {
            p.view.stopLoading()
            p.view.configuration.userContentController.removeAllScriptMessageHandlers()
            p.view.removeFromSuperview()
            pages[k] = nil
        }
    }

    func has(_ key: String) -> Bool { pages[key] != nil }
    func generation(_ key: String) -> Int? { pages[key]?.generation }

    func nav(_ key: String) -> (back: Bool, forward: Bool) {
        guard let v = pages[key]?.view else { return (false, false) }
        return (v.canGoBack, v.canGoForward)
    }

    /// Steps the view's history; the address it goes to, or why not.
    func step(_ key: String, back: Bool) -> Result<String, VornRequestError> {
        guard let v = pages[key]?.view else { return .failure(VornRequestError(BrowserBridge.noPane)) }
        let item = back ? v.backForwardList.backItem : v.backForwardList.forwardItem
        guard let item else {
            return .failure(VornRequestError(back ? "No page to go back to \u{2014} this is the first page in this tab."
                : "No page to go forward to \u{2014} this is the newest page in this tab."))
        }
        v.go(to: item)
        return .success(item.url.absoluteString)
    }

    /// Loads `url` even when it is the address last asked for.
    func load(_ key: String, _ url: String, root: String?) {
        guard let p = pages[key] else { return }
        p.asked = url
        Self.load(p.view, url, root: root)
    }

    func reload(_ key: String) { pages[key]?.view.reload() }
    func stop(_ key: String) { pages[key]?.view.stopLoading() }

    /// Runs `js` in the view, not waiting for an answer.
    func run(_ key: String, _ js: String) {
        pages[key]?.view.evaluateJavaScript(js + ";void 0", completionHandler: nil)
    }

    /// Runs `js` in the view and reads its JSON answer, giving up after ten seconds.
    func eval(_ key: String, _ js: String) async -> Result<JSONValue, VornRequestError> {
        guard let v = pages[key]?.view else { return .failure(VornRequestError(BrowserBridge.noPane)) }
        return await withCheckedContinuation { (cont: CheckedContinuation<Result<JSONValue, VornRequestError>, Never>) in
            let once = Once(cont)
            v.evaluateJavaScript(js) { value, _ in once.finish(BrowserBridge.pageAnswer(value as? String)) }
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(10))
                once.finish(.failure(VornRequestError(BrowserBridge.timedOut)))
            }
        }
    }
}

/// Resumes a continuation once, whichever answer comes first.
@MainActor
private final class Once {
    private var cont: CheckedContinuation<Result<JSONValue, VornRequestError>, Never>?
    init(_ cont: CheckedContinuation<Result<JSONValue, VornRequestError>, Never>) { self.cont = cont }
    func finish(_ r: Result<JSONValue, VornRequestError>) {
        cont?.resume(returning: r)
        cont = nil
    }
}
