import AppKit
import Observation
import SwiftUI

/// Toast.tsx: the window's stack of short messages, some with actions, some waiting on work.
@MainActor
@Observable
public final class ToastCenter {
    public enum Kind: Sendable { case success, error, warning, info, loading }

    public struct Action {
        public let label: String
        public var danger = false
        public let run: (String) -> Void
        public init(_ label: String, danger: Bool = false, run: @escaping (String) -> Void) {
            self.label = label
            self.danger = danger
            self.run = run
        }
    }

    public struct Item: Identifiable {
        public let id: String
        public var message: String
        public var kind: Kind
        public var actions: [Action]
    }

    public private(set) var items: [Item] = []
    @ObservationIgnored private var timers: [String: Task<Void, Never>] = [:]

    public init() {}

    static func duration(_ kind: Kind) -> Double? {
        switch kind {
        case .success, .info: 2.5
        case .error: 4
        case .warning: 3.5
        case .loading: nil
        }
    }

    /// Shows a toast; a nil `duration` with actions, or a loading one, stays until dismissed.
    @discardableResult
    public func show(_ message: String, _ kind: Kind = .success, actions: [Action] = [], sticky: Bool = false) -> String {
        let id = UUID().uuidString
        items.append(Item(id: id, message: message, kind: kind, actions: actions))
        schedule(id, sticky ? nil : Self.duration(kind))
        return id
    }

    /// Replaces a toast in place; a missing one is shown anew unless `onlyIfShown`.
    @discardableResult
    public func update(_ id: String, _ message: String, _ kind: Kind, actions: [Action]? = nil, sticky: Bool = false,
                       onlyIfShown: Bool = false) -> String? {
        guard let i = items.firstIndex(where: { $0.id == id }) else {
            return onlyIfShown ? nil : show(message, kind, actions: actions ?? [], sticky: sticky)
        }
        items[i].message = message
        items[i].kind = kind
        if let actions { items[i].actions = actions }
        schedule(id, sticky ? nil : Self.duration(kind))
        return id
    }

    public func dismiss(_ id: String) {
        timers[id]?.cancel()
        timers[id] = nil
        items.removeAll { $0.id == id }
    }

    private func schedule(_ id: String, _ seconds: Double?) {
        timers[id]?.cancel()
        guard let seconds else { return }
        timers[id] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            self?.dismiss(id)
        }
    }

    /// progress-toast.ts: a loading toast that turns into the outcome.
    @discardableResult
    public func progress<T>(loading: String, success: String, _ work: () async throws -> T) async -> T? {
        let id = show(loading, .loading)
        do {
            let r = try await work()
            update(id, success, .success)
            return r
        } catch {
            let msg = error.localizedDescription
            update(id, msg.isEmpty ? "Operation failed" : msg, .error)
            return nil
        }
    }
}

/// ToastContainer: bottom right, newest last.
public struct ToastStack: View {
    let center: ToastCenter

    public init(center: ToastCenter) { self.center = center }

    public var body: some View {
        VStack(alignment: .trailing, spacing: 8) {
            ForEach(center.items) { t in
                ToastRow(item: t) { center.dismiss(t.id) }
                    .transition(.asymmetric(insertion: .move(edge: .bottom).combined(with: .opacity), removal: .opacity))
            }
        }
        .animation(.spring(response: 0.25, dampingFraction: 0.8), value: center.items.map(\.id))
        .padding(.trailing, 20)
        .padding(.bottom, 20)
    }
}

private struct ToastRow: View {
    let item: ToastCenter.Item
    let onDismiss: () -> Void

    private var style: (icon: LucideGlyph, tint: Color, bg: Color, border: Color) {
        switch item.kind {
        case .success: (.check, Theme.green400, Theme.green500.opacity(0.1), Theme.green500.opacity(0.2))
        case .error: (.x, Theme.red400, Theme.red500.opacity(0.1), Theme.red500.opacity(0.2))
        case .warning: (.triangleAlert, Theme.amber400, Theme.amber500.opacity(0.1), Theme.amber500.opacity(0.2))
        case .info: (.info, Theme.blue400, Theme.blue500.opacity(0.1), Theme.blue500.opacity(0.2))
        case .loading: (.loaderCircle, Theme.gray300, Theme.white(0.05), Theme.white(0.1))
        }
    }

    /// The width the row would take unwrapped, capped by the 360 maximum.
    static func idealWidth(_ item: ToastCenter.Item) -> CGFloat {
        let text = (item.message as NSString).size(withAttributes: [.font: NSFont.systemFont(ofSize: 14)]).width
        let actions = item.actions.reduce(CGFloat(0)) { $0 + ($1.label as NSString).size(withAttributes: [.font: NSFont.systemFont(ofSize: 12)]).width + 26 }
        return min(360, 32 + 15 + 10 + text + 10 + actions + 16)
    }

    var body: some View {
        let s = style
        HStack(spacing: 10) {
            Group {
                if item.kind == .loading {
                    SpinningGlyph(glyph: s.icon, size: 15, strokeWidth: 2.5)
                } else {
                    LucideIcon(s.icon, size: 15, strokeWidth: 2.5)
                }
            }
            .foregroundStyle(s.tint)
            Text(item.message).font(.ui(14)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            ForEach(Array(item.actions.enumerated()), id: \.offset) { _, a in
                Hovering { hover in
                    Button { a.run(item.id) } label: {
                        Text(a.label).font(.ui(12))
                            .foregroundStyle(a.danger ? (hover ? Theme.red300 : Theme.red400) : (hover ? .white : Theme.gray300))
                            .padding(.horizontal, 8).padding(.vertical, 2)
                            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
                    }
                    .buttonStyle(.plain)
                }
            }
            IconButton(.x, size: 12, padding: 2, color: Theme.gray500, hoverColor: Theme.gray300, action: onDismiss)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .frame(minWidth: 200, idealWidth: Self.idealWidth(item), maxWidth: 360)
        .fixedSize(horizontal: true, vertical: false)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Color(red: 26 / 255, green: 26 / 255, blue: 30 / 255, opacity: 0.92)))
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(s.bg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(s.border, lineWidth: 1))
        .shadow(color: .black.opacity(0.25), radius: 12, y: 10)
    }
}

/// A glyph turning once a second while `active`, as `animate-spin` on Loader2 or a refreshing RefreshCw.
public struct SpinningGlyph: View {
    let glyph: LucideGlyph
    var size: CGFloat
    var strokeWidth: CGFloat
    var active: Bool
    @State private var angle = 0.0

    public init(glyph: LucideGlyph = .loaderCircle, size: CGFloat = 13, strokeWidth: CGFloat = 2, active: Bool = true) {
        self.glyph = glyph
        self.size = size
        self.strokeWidth = strokeWidth
        self.active = active
    }

    public var body: some View {
        LucideIcon(glyph, size: size, strokeWidth: strokeWidth)
            .rotationEffect(.degrees(active ? angle : 0))
            .onAppear(perform: spin)
            .onChange(of: active) { spin() }
    }

    private func spin() {
        guard active else { return }
        angle = 0
        withAnimation(.linear(duration: 1).repeatForever(autoreverses: false)) { angle = 360 }
    }
}
