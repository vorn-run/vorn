import SwiftUI

/// Wraps children onto new lines (`flex flex-wrap`).
public struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    public init(spacing: CGFloat = 6) { self.spacing = spacing }

    public func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews)
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: rows.last.map { $0.y + $0.height } ?? 0)
    }

    public func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var i = 0
        for row in arrange(width: bounds.width, subviews) {
            var x = bounds.minX
            for _ in 0..<row.count {
                let s = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + row.y), proposal: .unspecified)
                x += s.width + spacing
                i += 1
            }
        }
    }

    private func arrange(width: CGFloat, _ subviews: Subviews) -> [(y: CGFloat, height: CGFloat, width: CGFloat, count: Int)] {
        var rows: [(y: CGFloat, height: CGFloat, width: CGFloat, count: Int)] = []
        var x: CGFloat = 0, y: CGFloat = 0, h: CGFloat = 0, n = 0
        for v in subviews {
            let s = v.sizeThatFits(.unspecified)
            if n > 0 && x + s.width > width {
                rows.append((y, h, x - spacing, n))
                y += h + spacing
                x = 0; h = 0; n = 0
            }
            x += s.width + spacing
            h = max(h, s.height)
            n += 1
        }
        if n > 0 { rows.append((y, h, x - spacing, n)) }
        return rows
    }
}
