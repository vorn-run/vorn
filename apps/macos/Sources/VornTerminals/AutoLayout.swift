import Foundation

/// The smart-auto grid's choice of columns and rows (auto-grid-layout.ts).
public struct AutoLayout: Equatable, Sendable {
    public enum Mode: Sendable { case fit, scroll }
    public var cols: Int
    public var rows: Int
    public var mode: Mode

    public init(cols: Int, rows: Int, mode: Mode) {
        self.cols = cols
        self.rows = rows
        self.mode = mode
    }

    public static let minCardWidth: Double = 320
    public static let minCardHeight: Double = 200
    public static let rowFitMinHeight: Double = 280
    public static let targetAspect: Double = 1
    public static let hardMaxCols = 4
    public static let hardMaxRows = 4

    /// Rows that fit `height` before the grid scrolls.
    public static func fitMaxRows(_ height: Double) -> Int {
        clamp(Int(floor(height / rowFitMinHeight)), 1, hardMaxRows)
    }

    /// The row height of a scrolling grid.
    public static func scrollRowHeight(_ height: Double) -> Double {
        max(minCardHeight, floor(height / Double(fitMaxRows(height))))
    }

    /// The layout for `n` cards in a `width` by `height` area: nearest to square cards, few empty cells.
    public static func pick(_ n: Int, width: Double, height: Double) -> AutoLayout {
        if n <= 1 { return AutoLayout(cols: 1, rows: 1, mode: .fit) }
        if n == 2 && width / 2 >= minCardWidth { return AutoLayout(cols: 2, rows: 1, mode: .fit) }
        if n == 3 && width / 3 >= minCardWidth { return AutoLayout(cols: 3, rows: 1, mode: .fit) }
        let maxCols = clamp(Int(floor(width / minCardWidth)), 1, hardMaxCols)
        let maxRows = clamp(Int(floor(height / rowFitMinHeight)), 1, hardMaxRows)
        if n > maxCols * maxRows {
            let cols = clamp(Int(floor(width / minCardWidth)), 1, maxCols)
            return AutoLayout(cols: cols, rows: (n + cols - 1) / cols, mode: .scroll)
        }
        var best = (cols: 1, rows: n, score: -Double.infinity)
        for cols in 1...min(n, maxCols) {
            let rows = (n + cols - 1) / cols
            if rows > maxRows { continue }
            let aspectPenalty = abs(log((width / Double(cols)) / (height / Double(rows)) / targetAspect))
            let emptyPenalty = Double(cols * rows - n) * 0.25
            let score = -aspectPenalty - emptyPenalty
            if score > best.score || (score == best.score && cols > best.cols) {
                best = (cols, rows, score)
            }
        }
        return AutoLayout(cols: best.cols, rows: best.rows, mode: .fit)
    }

    private static func clamp(_ v: Int, _ lo: Int, _ hi: Int) -> Int { min(max(v, lo), hi) }
}
