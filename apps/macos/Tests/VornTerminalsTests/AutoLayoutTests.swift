import Testing
@testable import VornTerminals

struct AutoLayoutTests {
    @Test func singleCardFills() {
        #expect(AutoLayout.pick(1, width: 1000, height: 700) == AutoLayout(cols: 1, rows: 1, mode: .fit))
    }

    @Test func twoAndThreeSitSideBySideWhenWideEnough() {
        #expect(AutoLayout.pick(2, width: 1000, height: 700) == AutoLayout(cols: 2, rows: 1, mode: .fit))
        #expect(AutoLayout.pick(3, width: 1000, height: 700) == AutoLayout(cols: 3, rows: 1, mode: .fit))
        #expect(AutoLayout.pick(3, width: 900, height: 700).cols < 3)
    }

    @Test func fourIsTwoByTwo() {
        #expect(AutoLayout.pick(4, width: 1200, height: 800) == AutoLayout(cols: 2, rows: 2, mode: .fit))
    }

    @Test func overflowScrolls() {
        let l = AutoLayout.pick(20, width: 1200, height: 600)
        #expect(l.mode == .scroll)
        #expect(l.cols == 3)
        #expect(l.rows == 7)
        #expect(AutoLayout.scrollRowHeight(600) == 300)
    }
}
