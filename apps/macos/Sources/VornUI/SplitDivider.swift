import SwiftUI

/// split-ratio.ts: what a divider may set, and how stacked panes share one column.
public enum SplitRatio {
    public static let min: Double = 0.15
    public static let max: Double = 0.85
    public static let `default`: Double = 0.5

    public static func clamp(_ r: Double) -> Double { r.isFinite ? Swift.max(min, Swift.min(max, r)) : `default` }

    /// splitPaneWeights: stored weights for `count` panes, normalised; missing ones even.
    public static func weights(_ stored: [Double]?, count: Int) -> [Double] {
        guard count > 0 else { return [] }
        let even = 1 / Double(count)
        let raw = (0..<count).map { i -> Double in
            if let n = stored?[safe: i], n.isFinite, n > 0 { return n }
            return even
        }
        let total = raw.reduce(0, +)
        return total > 0 ? raw.map { $0 / total } : raw.map { _ in even }
    }

    /// resizePaneWeights: the divider below pane `index` moved to `ratio` of the column.
    public static func resize(_ weights: [Double], index: Int, ratio: Double) -> [Double] {
        guard index >= 0, index + 1 < weights.count else { return weights }
        let before = weights[..<index].reduce(0, +)
        let pair = weights[index] + weights[index + 1]
        guard pair > 0 else { return weights }
        let target = ratio.isFinite ? ratio - before : pair / 2
        let first = Swift.max(min * pair, Swift.min(max * pair, target))
        var next = weights
        next[index] = first
        next[index + 1] = pair - first
        return next
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

/// SplitDivider.tsx: a 1px rule that drags; `onChange` while moving, `onCommit` once on release.
public struct SplitDivider: View {
    public enum Axis { case x, y }
    let axis: Axis
    /// The container's extent along the axis, measured when the drag starts.
    let extent: () -> CGFloat
    let origin: () -> CGFloat
    let onChange: (Double) -> Void
    let onCommit: (Double) -> Void
    let label: String
    @State private var hover = false
    @State private var start: (origin: CGFloat, extent: CGFloat)?
    @State private var last: Double?

    public init(axis: Axis, label: String, origin: @escaping () -> CGFloat = { 0 }, extent: @escaping () -> CGFloat,
                onChange: @escaping (Double) -> Void, onCommit: @escaping (Double) -> Void) {
        self.axis = axis
        self.label = label
        self.origin = origin
        self.extent = extent
        self.onChange = onChange
        self.onCommit = onCommit
    }

    public var body: some View {
        Rectangle()
            .fill(hover || start != nil ? Theme.blue500.opacity(0.4) : Theme.white(0.06))
            .frame(width: axis == .x ? 1 : nil, height: axis == .y ? 1 : nil)
            .overlay {
                Color.clear
                    .frame(width: axis == .x ? 7 : nil, height: axis == .y ? 7 : nil)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        hover = inside
                        if inside { (axis == .x ? NSCursor.resizeLeftRight : NSCursor.resizeUpDown).push() } else { NSCursor.pop() }
                    }
                    .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global)
                        .onChanged { g in
                            if start == nil { start = (origin(), extent()) }
                            guard let s = start, s.extent > 0 else { return }
                            let pos = axis == .x ? g.location.x : g.location.y
                            let r = SplitRatio.clamp(Double((pos - s.origin) / s.extent))
                            last = r
                            onChange(r)
                        }
                        .onEnded { _ in
                            if let last { onCommit(last) }
                            start = nil
                            last = nil
                        })
            }
            .accessibilityLabel(label)
    }
}
