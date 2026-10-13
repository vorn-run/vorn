import Testing
@testable import VornTerminals

/// The cells a drag selects, as the painter fills them.
struct TerminalSelectionTests {
    private func p(_ line: UInt64, _ col: UInt16) -> GridPoint { GridPoint(line: line, col: col) }

    @Test func orderDoesNotDependOnDragDirection() {
        #expect(TerminalSelection(p(5, 3), p(2, 7)) == TerminalSelection(p(2, 7), p(5, 3)))
        #expect(TerminalSelection(p(4, 4), p(4, 4)).isEmpty)
    }

    @Test func columnsCoverTheMiddleLinesWhole() {
        let s = TerminalSelection(p(2, 7), p(4, 3))
        #expect(s.cols(on: 1, width: 80) == nil)
        #expect(s.cols(on: 2, width: 80) == 7..<80)
        #expect(s.cols(on: 3, width: 80) == 0..<80)
        #expect(s.cols(on: 4, width: 80) == 0..<3)
        #expect(s.cols(on: 5, width: 80) == nil)
        #expect(TerminalSelection(p(1, 2), p(1, 6)).cols(on: 1, width: 4) == 2..<4)
        #expect(TerminalSelection(p(1, 0), p(2, 0)).cols(on: 2, width: 80) == nil)
    }
}
