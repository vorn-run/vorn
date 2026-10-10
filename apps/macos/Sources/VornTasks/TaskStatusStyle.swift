import SwiftUI
import VornCore
import VornUI

/// lib/task-status.ts: one place for each status's words, glyph and colours.
extension TaskStatus {
    static let boardOrder: [TaskStatus] = [.todo, .inProgress, .inReview, .done, .cancelled]

    var label: String {
        switch self {
        case .todo: "Todo"
        case .inProgress: "In Progress"
        case .inReview: "In Review"
        case .done: "Done"
        case .cancelled: "Cancelled"
        }
    }

    var glyph: LucideGlyph {
        switch self {
        case .todo: .circle
        case .inProgress: .clock
        case .inReview: .eye
        case .done: .circleCheck
        case .cancelled: .circleX
        }
    }

    var tint: Color {
        switch self {
        case .todo: Theme.statusSlate
        case .inProgress: Theme.statusBlue
        case .inReview: Theme.bronzo
        case .done: Theme.statusSage
        case .cancelled: Theme.inkFaint
        }
    }

    var dot: Color {
        self == .cancelled ? Theme.inkGhost : tint
    }

    var emptyText: String {
        switch self {
        case .todo: "No tasks in queue"
        case .inProgress: "No active tasks"
        case .inReview: "No tasks awaiting review"
        case .done: "No completed tasks"
        case .cancelled: "No cancelled tasks"
        }
    }
}

struct StatusIcon: View {
    let status: TaskStatus
    var size: CGFloat = 14

    var body: some View {
        LucideIcon(status.glyph, size: size).foregroundStyle(status.tint)
    }
}

enum TaskDates {
    private static let iso: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let isoPlain = ISO8601DateFormatter()

    static func parse(_ s: String) -> Date? {
        iso.date(from: s) ?? isoPlain.date(from: s)
    }

    /// `formatTaskDate`: "Oct 9".
    static func short(_ s: String) -> String {
        guard let d = parse(s) else { return "" }
        return d.formatted(.dateTime.locale(Locale(identifier: "en_US")).month(.abbreviated).day())
    }

    /// The detail panel's date: month, day and a two-digit time in the user's locale.
    static func long(_ s: String) -> String {
        guard let d = parse(s) else { return "" }
        return d.formatted(.dateTime.month(.abbreviated).day().hour(.twoDigits(amPM: .abbreviated)).minute(.twoDigits))
    }
}
