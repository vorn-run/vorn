import AppKit
import VornUI

/// The find bar a terminal shows over its top-right corner: a query field, the
/// match count and previous, next and close buttons.
final class TerminalFindBar: NSView, NSTextFieldDelegate {
    let field = NSTextField()
    private let count = NSTextField(labelWithString: "")
    var onQuery: ((String) -> Void)?
    var onStep: ((Int) -> Void)?
    var onClose: (() -> Void)?

    static let size = NSSize(width: 288, height: 32)

    init() {
        super.init(frame: NSRect(origin: .zero, size: Self.size))
        wantsLayer = true
        layer?.backgroundColor = NSColor(Theme.surfaceOverlay).cgColor
        layer?.cornerRadius = Theme.radius
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.white.withAlphaComponent(0.08).cgColor
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.35)
        shadow.shadowOffset = NSSize(width: 0, height: -4)
        shadow.shadowBlurRadius = 16
        self.shadow = shadow

        field.placeholderString = "Find"
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 12)
        field.textColor = NSColor(Theme.ink)
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.delegate = self
        field.setAccessibilityLabel("Find in terminal")

        count.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        count.textColor = NSColor(Theme.inkSecondary)
        count.alignment = .right

        let previous = button("chevron.up", "Previous match", #selector(previousMatch))
        let next = button("chevron.down", "Next match", #selector(nextMatch))
        let close = button("xmark", "Close find", #selector(close))
        let stack = NSStackView(views: [field, count, previous, next, close])
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 4)
        stack.translatesAutoresizingMaskIntoConstraints = false
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        count.setContentHuggingPriority(.required, for: .horizontal)
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    private func button(_ symbol: String, _ label: String, _ action: Selector) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)?
            .withSymbolConfiguration(.init(pointSize: 11, weight: .medium))
        let b = NSButton(image: image ?? NSImage(), target: self, action: action)
        b.isBordered = false
        b.contentTintColor = NSColor(Theme.inkSecondary)
        b.toolTip = label
        b.setAccessibilityLabel(label)
        b.widthAnchor.constraint(equalToConstant: 24).isActive = true
        b.heightAnchor.constraint(equalToConstant: 24).isActive = true
        return b
    }

    /// "3 of 12", "No results", or nothing before a query.
    func show(current: Int?, total: Int, query: String) {
        count.stringValue = query.isEmpty ? "" : total == 0 ? "No results"
            : current.map { "\($0 + 1) of \(total)" } ?? "\(total)"
    }

    var countText: String { count.stringValue }

    @objc private func previousMatch() { onStep?(-1) }
    @objc private func nextMatch() { onStep?(1) }
    @objc private func close() { onClose?() }

    func controlTextDidChange(_ n: Notification) { onQuery?(field.stringValue) }

    /// Return finds the next match, Shift-Return the previous, Esc closes.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy s: Selector) -> Bool {
        switch s {
        case #selector(insertNewline(_:)):
            onStep?(NSApp.currentEvent?.modifierFlags.contains(.shift) == true ? -1 : 1)
        case #selector(cancelOperation(_:)):
            onClose?()
        default:
            return false
        }
        return true
    }
}
