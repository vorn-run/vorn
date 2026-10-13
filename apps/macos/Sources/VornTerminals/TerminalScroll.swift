import Foundation

/// A terminal's scroll position over its history, as a Mac scrolls: a
/// trackpad moves it by the point, momentum carries it on, it stretches past
/// either end and springs back, and it comes to rest on a whole row. A
/// notched wheel moves `notchRows` rows with a short ease. The same model as
/// Vorn Native's (vorn-app/src/scroll.rs).
///
/// The position is points above the live bottom: 0 is the live screen, `max`
/// the oldest line of history. `dy` is positive towards the live bottom.
struct TerminalScroll: Equatable {
    enum Phase { case line, began, moved, ended }

    static let notchRows: CGFloat = 3
    /// A touch that lifts and a `began` this soon after is the momentum.
    static let momentumGap: TimeInterval = 0.060
    /// Momentum that never says it ended settles once it stops this long.
    static let idle: TimeInterval = 0.150
    static let ease: TimeInterval = 0.120
    /// The spring that settles on a row, and the softer one back from an edge.
    static let snap: TimeInterval = 0.200
    static let bounce: TimeInterval = 0.300
    /// How stiff the stretch past an edge is.
    static let band: CGFloat = 0.55

    enum Motion: Equatable {
        case rest
        /// Fingers on the trackpad; `raw` is where they would put it unstretched.
        case touch(raw: CGFloat)
        case lifted(at: TimeInterval)
        case momentum(at: TimeInterval)
        case free(at: TimeInterval)
        case spring(x0: CGFloat, v0: CGFloat, to: CGFloat, start: TimeInterval, dur: TimeInterval)
        case ease(from: CGFloat, to: CGFloat, start: TimeInterval)
    }

    private(set) var pos: CGFloat = 0
    private(set) var max: CGFloat = 0
    private(set) var row: CGFloat = 1
    private var span: CGFloat = 1
    /// Points a second, positive into history.
    private var vel: CGFloat = 0
    private var last: TimeInterval?
    private(set) var motion = Motion.rest
    /// A spring took over at an edge: the rest of the momentum is ignored.
    private var spent = false
    /// Reduce motion: a notch jumps instead of easing.
    var reduced = false

    var resting: Bool { motion == .rest }

    /// Whether it rests away from the live screen, or is headed there.
    var scrolled: Bool { pos > 0.5 || target > 0.5 }

    /// Row height, pane height and history rows.
    mutating func setMetrics(row r: CGFloat, span s: CGFloat, historyRows: UInt64) {
        let r = Swift.max(r, 1)
        if r != row, motion == .rest { pos = (pos / row).rounded() * r }
        row = r
        span = Swift.max(s, r)
        max = CGFloat(historyRows) * r
        if motion == .rest { pos = Swift.min(Swift.max(pos, 0), max) }
    }

    /// `rows` new lines went into history: a view scrolled back keeps showing
    /// the same lines; one at the live screen follows it.
    mutating func grew(_ rows: UInt64) {
        guard rows > 0, scrolled else { return }
        let px = CGFloat(rows) * row
        pos += px
        switch motion {
        case .touch(let raw): motion = .touch(raw: raw + px)
        case let .spring(x0, v0, to, start, dur): motion = .spring(x0: x0, v0: v0, to: to + px, start: start, dur: dur)
        case let .ease(from, to, start): motion = .ease(from: from + px, to: to + px, start: start)
        default: break
        }
    }

    /// Straight back to the live screen, as typing does.
    mutating func reset() {
        pos = 0
        vel = 0
        motion = .rest
    }

    /// A wheel of `dy` points, or `dy` notches for `.line`, at `now`.
    mutating func wheel(_ dy: CGFloat, _ phase: Phase, now: TimeInterval) {
        switch phase {
        case .line: notch(dy, now)
        case .began: began(now)
        case .moved: moved(dy, now)
        case .ended: ended(now)
        }
    }

    private mutating func notch(_ notches: CGFloat, _ now: TimeInterval) {
        let base: CGFloat
        if case .ease(_, let to, _) = motion { base = to } else { base = pos }
        let to = snapped(base - notches * Self.notchRows * row)
        if reduced { return restAt(to) }
        vel = 0
        motion = .ease(from: pos, to: to, start: now)
    }

    private mutating func began(_ now: TimeInterval) {
        if case .lifted(let at) = motion, now - at <= Self.momentumGap {
            if inside {
                motion = .momentum(at: now)
            } else {
                // Lifted past an edge: it springs back, the momentum is not wanted.
                spent = true
                settle(now)
            }
            return
        }
        if case .touch = motion { return }
        spent = false
        vel = 0
        last = now
        motion = .touch(raw: unstretch(pos))
    }

    private mutating func moved(_ dy: CGFloat, _ now: TimeInterval) {
        let dt = last.map { now - $0 } ?? 0
        last = now
        let v = dt > 0 ? -dy / CGFloat(Swift.max(dt, 0.004)) : vel
        switch motion {
        case .touch(let raw):
            let raw = raw - dy
            pos = stretch(raw)
            vel = 0.6 * v + 0.4 * vel
            motion = .touch(raw: raw)
        case .momentum:
            vel = 0.6 * v + 0.4 * vel
            let next = pos - dy
            if next >= 0, next <= max {
                pos = next
                motion = .momentum(at: now)
            } else {
                // Out over the edge with the momentum's speed, then back.
                pos = Swift.min(Swift.max(next, 0), max)
                spent = true
                springTo(pos, Self.bounce, now)
            }
        case .spring, .lifted:
            if spent { return }
            fallthrough
        default:
            pos = Swift.min(Swift.max(pos - dy, 0), max)
            vel = 0
            motion = .free(at: now)
        }
    }

    private mutating func ended(_ now: TimeInterval) {
        switch motion {
        case .touch:
            // Fingers that stopped before lifting throw nothing.
            if last.map({ now - $0 > 0.050 }) ?? true { vel = 0 }
            motion = .lifted(at: now)
        case .momentum:
            vel = 0
            settle(now)
        default: break
        }
        if case .lifted = motion {} else { spent = false }
    }

    /// Moves the animation to `now`; true while it needs more frames.
    @discardableResult
    mutating func tick(_ now: TimeInterval) -> Bool {
        switch motion {
        case .rest, .touch: return false
        case .lifted(let at):
            guard now - at >= Self.momentumGap else { return true }
            settle(now)
            return motion != .rest
        case .momentum(let at), .free(let at):
            guard now - at >= Self.idle else { return true }
            vel = 0
            settle(now)
            return motion != .rest
        case let .spring(x0, v0, to, start, dur):
            let t = now - start
            let (x, v) = Self.spring(dur, x0, v0, t)
            if (abs(x) < 0.5 && abs(v) < 20) || t > 1.5 {
                restAt(to)
                return false
            }
            pos = to + x
            vel = v
            return true
        case let .ease(from, to, start):
            let t = (now - start) / Self.ease
            if t >= 1 {
                restAt(to)
                return false
            }
            let e = 1 - pow(1 - CGFloat(t), 3)
            pos = from + (to - from) * e
            return true
        }
    }

    private mutating func restAt(_ to: CGFloat) {
        pos = to
        vel = 0
        motion = .rest
    }

    /// Springs to the nearest row within the history.
    private mutating func settle(_ now: TimeInterval) {
        let wasInside = inside
        let to = snapped(pos)
        if abs(pos - to) < 0.5, abs(vel) < 20 { return restAt(to) }
        if !wasInside || abs(vel) < 1 {
            motion = .spring(x0: pos - to, v0: wasInside ? 0 : vel, to: to, start: now,
                             dur: wasInside ? Self.snap : Self.bounce)
        } else {
            springTo(to, Self.snap, now)
        }
    }

    private mutating func springTo(_ to: CGFloat, _ dur: TimeInterval, _ now: TimeInterval) {
        motion = .spring(x0: pos - to, v0: vel, to: to, start: now, dur: dur)
    }

    private var target: CGFloat {
        switch motion {
        case .spring(_, _, let to, _, _), .ease(_, let to, _): return to
        default: return pos
        }
    }

    private var inside: Bool { pos >= 0 && pos <= max }

    private func snapped(_ px: CGFloat) -> CGFloat {
        Swift.min(Swift.max((px / row).rounded() * row, 0), max)
    }

    /// Where fingers at `raw` show it: past an edge, ever less far.
    private func stretch(_ raw: CGFloat) -> CGFloat {
        let band = { (x: CGFloat) in (1 - 1 / (x * Self.band / span + 1)) * span }
        if raw < 0 { return -band(-raw) }
        if raw > max { return max + band(raw - max) }
        return raw
    }

    private func unstretch(_ p: CGFloat) -> CGFloat {
        let unband = { (y: CGFloat) -> CGFloat in
            let y = Swift.min(y, span * 0.99)
            return span / Self.band * (1 / (1 - y / span) - 1)
        }
        if p < 0 { return -unband(-p) }
        if p > max { return max + unband(p - max) }
        return p
    }

    /// The rows back from the live screen the view starts at, and the offset
    /// to draw it at (positive moves it down).
    var split: (back: UInt64, dy: CGFloat) {
        let inside = Swift.min(Swift.max(pos, 0), max)
        let back = Swift.max((inside / row - 1e-3).rounded(.up), 0)
        return (UInt64(back), pos - back * row)
    }

    /// A unit-mass spring with no overshoot of its own and a period of `dur`:
    /// displacement and velocity `t` seconds after it was at `x0` moving at `v0`.
    static func spring(_ dur: TimeInterval, _ x0: CGFloat, _ v0: CGFloat, _ t: TimeInterval) -> (CGFloat, CGFloat) {
        let w0 = 2 * CGFloat.pi / CGFloat(Swift.max(dur, 1e-3))
        let t = CGFloat(t)
        let e = exp(-w0 * t)
        let b = v0 + w0 * x0
        return (e * (x0 + b * t), e * (b - w0 * (x0 + b * t)))
    }
}

/// Where a wheel goes: the program when it tracks the mouse, arrow keys on the
/// alternate screen, else the history.
enum WheelRoute: Equatable {
    case program, arrows, history

    init(mouse: Bool, alternate: Bool) {
        self = mouse ? .program : alternate ? .arrows : .history
    }
}

/// Whole rows out of trackpad travel for a program: the fraction carries.
struct WheelRows: Equatable {
    private var rest: CGFloat = 0

    /// Rows for `dy` points (or notches for `.line`) of `row` height, positive
    /// down; a notch is one wheel report to a program, three arrow presses.
    mutating func take(_ dy: CGFloat, _ phase: TerminalScroll.Phase, row: CGFloat, route: WheelRoute) -> Int32 {
        switch phase {
        case .line:
            rest = 0
            return Int32((dy * (route == .arrows ? TerminalScroll.notchRows : 1)).rounded())
        case .began, .ended:
            rest = 0
            return 0
        case .moved:
            let rows = rest + dy / Swift.max(row, 1)
            let whole = rows.rounded(.towardZero)
            rest = rows - whole
            return Int32(whole)
        }
    }
}
