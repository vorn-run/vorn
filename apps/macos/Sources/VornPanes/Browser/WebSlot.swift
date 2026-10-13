import AppKit
import SwiftUI
import WebKit

/// Holds one of the host's page views, reparented here so the page outlives the SwiftUI view that shows it.
struct WebSlot: NSViewRepresentable {
    let model: BrowserModel
    let key: String
    /// Changes when the slot should look for its view again.
    var token: String = ""
    var zoom: Double?
    /// Kept loaded but not drawn, while something covers it.
    var covered = false

    func makeNSView(context: Context) -> SlotView { SlotView() }

    func updateNSView(_ slot: SlotView, context: Context) {
        model.syncViews()
        slot.isHidden = covered
        slot.show(model.host.pages[key]?.view, zoom: zoom)
    }

    static func dismantleNSView(_ slot: SlotView, coordinator: ()) {
        slot.show(nil, zoom: nil)
    }

    final class SlotView: NSView {
        private weak var shown: WKWebView?

        func show(_ view: WKWebView?, zoom: Double?) {
            if let zoom, let view, view.pageZoom != zoom { view.pageZoom = zoom }
            guard shown !== view else { return }
            if let shown, shown.superview === self { shown.removeFromSuperview() }
            shown = view
            guard let view else { return }
            view.removeFromSuperview()
            view.frame = bounds
            view.autoresizingMask = [.width, .height]
            addSubview(view)
        }

        override func layout() {
            super.layout()
            shown?.frame = bounds
        }
    }
}
