import AppKit
import Foundation
import Testing
@testable import VornTerminals

/// The scroll model: by the point under a touch, momentum, the stretch past
/// either end, rows to rest on, and where a wheel goes.
struct TerminalScrollTests {
    private let row: CGFloat = 20

    /// 100 rows of history in a 400 pt pane.
    private func scroll(history: UInt64 = 100) -> TerminalScroll {
        var s = TerminalScroll()
        s.setMetrics(row: row, span: 400, historyRows: history)
        return s
    }

    private func ms(_ n: Int) -> TimeInterval { TimeInterval(n) / 1000 }

    /// Ticks every 8 ms from `from` until it rests.
    @discardableResult
    private func run(_ s: inout TerminalScroll, from: Int) -> Int {
        var t = from
        while s.tick(ms(t)) {
            t += 8
            if t > from + 3000 { Issue.record("never settled: \(s)"); break }
        }
        return t
    }

    @Test func aTouchMovesByThePointAndSettlesOnARow() {
        var s = scroll()
        s.wheel(0, .began, now: 0)
        s.wheel(-7, .moved, now: ms(8))
        s.wheel(-6, .moved, now: ms(16))
        #expect(s.pos == 13)
        s.wheel(0, .ended, now: ms(24))
        // Still at the point while momentum may come.
        let waiting = s.tick(ms(30))
        #expect(waiting)
        #expect(s.pos == 13)
        run(&s, from: 90)
        #expect(s.pos == row)
        #expect(s.resting)
    }

    @Test func momentumCarriesOnAfterTheTouch() {
        var s = scroll()
        s.wheel(0, .began, now: 0)
        s.wheel(-30, .moved, now: ms(8))
        s.wheel(0, .ended, now: ms(16))
        s.wheel(0, .began, now: ms(24))
        for i in 0..<10 { s.wheel(-10, .moved, now: ms(32 + i * 8)) }
        #expect(s.pos == 130)
        s.wheel(0, .ended, now: ms(120))
        run(&s, from: 128)
        #expect(s.pos == 140)
    }

    @Test func aLateBeganIsANewTouchNotMomentum() {
        var s = scroll()
        s.wheel(0, .began, now: 0)
        s.wheel(-40, .moved, now: ms(8))
        s.wheel(0, .ended, now: ms(16))
        run(&s, from: 100)
        s.wheel(0, .began, now: ms(400))
        guard case .touch = s.motion else { Issue.record("\(s.motion)"); return }
    }

    @Test func pullingPastTheLiveBottomStretchesLessAndSpringsBack() {
        var s = scroll()
        s.wheel(0, .began, now: 0)
        s.wheel(100, .moved, now: ms(8))
        let p1 = s.pos
        #expect(p1 < 0 && p1 > -100)
        s.wheel(100, .moved, now: ms(16))
        let p2 = s.pos
        #expect(p1 - p2 < -p1, "each point pulls less")
        #expect(p2 > -400)
        s.wheel(0, .ended, now: ms(24))
        run(&s, from: 32)
        #expect(s.pos == 0)
    }

    @Test func momentumOverTheTopOvershootsThenSpringsBack() {
        var s = scroll(history: 2)
        s.wheel(0, .began, now: 0)
        s.wheel(-30, .moved, now: ms(8))
        s.wheel(0, .ended, now: ms(16))
        s.wheel(0, .began, now: ms(20))
        s.wheel(-20, .moved, now: ms(28))
        #expect(s.pos == 40)
        guard case .spring = s.motion else { Issue.record("\(s.motion)"); return }
        // Further momentum is ignored while the spring has it.
        s.wheel(-20, .moved, now: ms(36))
        s.tick(ms(60))
        #expect(s.pos > 40)
        run(&s, from: 60)
        #expect(s.pos == 40)
    }

    @Test func aNotchEasesThreeRows() {
        var s = scroll()
        s.wheel(-1, .line, now: 0)
        s.tick(ms(40))
        #expect(s.pos > 0 && s.pos < 3 * row)
        // A second notch mid-ease aims past the first's end.
        s.wheel(-1, .line, now: ms(40))
        run(&s, from: 48)
        #expect(s.pos == 6 * row)
        s.wheel(9, .line, now: ms(500))
        run(&s, from: 500)
        #expect(s.pos == 0)
    }

    @Test func reducedMotionJumpsANotch() {
        var s = scroll()
        s.reduced = true
        s.wheel(-1, .line, now: 0)
        #expect(s.pos == 3 * row && s.resting)
    }

    @Test func phaselessPointsClampAndSettleWhenIdle() {
        var s = scroll()
        s.wheel(-25, .moved, now: 0)
        #expect(s.pos == 25)
        s.wheel(100, .moved, now: ms(8))
        #expect(s.pos == 0)
        s.wheel(-27, .moved, now: ms(16))
        let waiting = s.tick(ms(20))
        #expect(waiting)
        run(&s, from: 200)
        #expect(s.pos == row)
    }

    @Test func newOutputKeepsAScrolledViewOnItsLines() {
        var s = scroll()
        s.wheel(-1, .line, now: 0)
        run(&s, from: 8)
        s.setMetrics(row: row, span: 400, historyRows: 105)
        s.grew(5)
        #expect(s.pos == 8 * row)
        var live = scroll()
        live.grew(5)
        #expect(live.pos == 0)
    }

    @Test func splitGivesRowsBackAndAnOffset() {
        var s = scroll()
        #expect(s.split == (0, 0))
        s.wheel(-45, .moved, now: 0)
        #expect(s.split == (3, -15))
        s.wheel(5, .moved, now: ms(8))
        #expect(s.split == (2, 0))
    }

    @Test func typingReturnsToTheLiveScreen() {
        var s = scroll()
        s.wheel(-1, .line, now: 0)
        s.reset()
        #expect(s.pos == 0 && s.resting && !s.scrolled)
    }

    @Test func wheelsGoToTheProgramThenArrowsThenHistory() {
        #expect(WheelRoute(mouse: true, alternate: true) == .program)
        #expect(WheelRoute(mouse: true, alternate: false) == .program)
        #expect(WheelRoute(mouse: false, alternate: true) == .arrows)
        #expect(WheelRoute(mouse: false, alternate: false) == .history)
    }

    @Test func trackpadTravelReachesAProgramARowAtATime() {
        var r = WheelRows()
        #expect(r.take(-12, .moved, row: row, route: .program) == 0)
        #expect(r.take(-12, .moved, row: row, route: .program) == -1)
        #expect(r.take(-50, .moved, row: row, route: .program) == -2)
        #expect(r.take(0, .ended, row: row, route: .program) == 0)
        #expect(r.take(-12, .moved, row: row, route: .program) == 0)
        #expect(r.take(1, .line, row: row, route: .program) == 1)
        #expect(r.take(-1, .line, row: row, route: .arrows) == -3)
    }

    @Test func readsTouchAndMomentumPhasesFromTheEvent() throws {
        func event(_ units: CGScrollEventUnit, _ dy: Int32, scroll: Int64 = 0, momentum: Int64 = 0) throws -> NSEvent {
            let e = try #require(CGEvent(scrollWheelEvent2Source: nil, units: units, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0))
            if units == .pixel { e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1) }
            e.setIntegerValueField(.scrollWheelEventScrollPhase, value: scroll)
            e.setIntegerValueField(.scrollWheelEventMomentumPhase, value: momentum)
            return try #require(NSEvent(cgEvent: e))
        }
        #expect(TerminalNSView.phase(of: try event(.line, 1)) == .line)
        #expect(TerminalNSView.phase(of: try event(.pixel, 0, scroll: 1)) == .began)
        let moved = try event(.pixel, 7, scroll: 2)
        #expect(TerminalNSView.phase(of: moved) == .moved)
        #expect(moved.scrollingDeltaY == 7)
        #expect(TerminalNSView.phase(of: try event(.pixel, 0, scroll: 4)) == .ended)
        #expect(TerminalNSView.phase(of: try event(.pixel, 9, momentum: 1)) == .began)
        #expect(TerminalNSView.phase(of: try event(.pixel, 9, momentum: 2)) == .moved)
        #expect(TerminalNSView.phase(of: try event(.pixel, 0, momentum: 3)) == .ended)
    }
}
