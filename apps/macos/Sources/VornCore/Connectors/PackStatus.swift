import Foundation

/// status-tone.ts: how loudly a state reads.
public enum StatusTone: String, Hashable, Sendable {
    case live, blocked, broken, settled, idle
}

/// pack-status.ts PackState: where a pack stands on this machine.
public enum PackState: Hashable, Sendable {
    case absent
    case notReleased
    case installing(phase: InstallProgress.Phase, percent: Double?)
    case installed(version: String, availableVersion: String?, previousVersion: String?)
    case rejected(error: String)

    public init(installed: InstalledPack?, catalogItem: CatalogItem?, progress: InstallProgress?) {
        let released = catalogItem == nil || catalogItem?.packUrl?.isEmpty == false
        if let progress, progress.phase != .installed, progress.phase != .failed {
            self = .installing(phase: progress.phase, percent: progress.percent)
            return
        }
        if let progress, progress.phase == .failed {
            self = .rejected(error: progress.error ?? "The pack could not be installed")
            return
        }
        guard let installed else {
            self = released ? .absent : .notReleased
            return
        }
        let published = released ? catalogItem?.version : nil
        let available = published.flatMap { !$0.isEmpty && PackState.isNewerVersion($0, than: installed.version) ? $0 : nil }
        self = .installed(version: installed.version, availableVersion: available, previousVersion: installed.previousVersion)
    }

    public var isInstalled: Bool {
        if case .installed = self { return true }
        return false
    }

    /// version-order.ts isNewerVersion: semver-ish, where a pre-release sorts before its release.
    public static func isNewerVersion(_ candidate: String, than current: String) -> Bool {
        enum Part: Equatable {
            case number(Int), text(String)
        }
        func split(_ version: String) -> (release: [Int], pre: [Part]) {
            let core = version.split(separator: "+", omittingEmptySubsequences: false).first.map(String.init) ?? ""
            let pieces = core.split(separator: "-", omittingEmptySubsequences: false).map(String.init)
            let release = (pieces.first ?? "").split(separator: ".", omittingEmptySubsequences: false).map { part in
                part.allSatisfy(\.isASCIIDigit) && !part.isEmpty ? Int(part) ?? 0 : 0
            }
            let pre = pieces.dropFirst().joined(separator: "-").split(separator: ".").map { part -> Part in
                part.allSatisfy(\.isASCIIDigit) ? .number(Int(part) ?? 0) : .text(String(part))
            }
            return (release, pre)
        }
        let left = split(candidate), right = split(current)
        for i in 0..<max(left.release.count, right.release.count) {
            let a = i < left.release.count ? left.release[i] : 0
            let b = i < right.release.count ? right.release[i] : 0
            if a != b { return a > b }
        }
        if left.pre.isEmpty { return !right.pre.isEmpty }
        if right.pre.isEmpty { return false }
        for i in 0..<max(left.pre.count, right.pre.count) {
            guard i < left.pre.count else { return false }
            guard i < right.pre.count else { return true }
            switch (left.pre[i], right.pre[i]) {
            case let (a, b) where a == b: continue
            case let (.number(a), .number(b)): return a > b
            case (.number, _): return false
            case (_, .number): return true
            case let (.text(a), .text(b)): return a > b
            }
        }
        return false
    }
}

private extension Character {
    var isASCIIDigit: Bool { isASCII && isNumber }
}

/// pack-status.ts PackStatusView: a pack state as one label, a line and an action.
public struct PackStatusView: Hashable, Sendable {
    public enum Action: String, Hashable, Sendable {
        case install, update, retry
    }
    public var label: String
    public var detail: String?
    public var tone: StatusTone
    public var percent: Double?
    public var action: Action?
    public var busy: Bool

    public init(_ state: PackState) {
        switch state {
        case .absent:
            self.init(label: "Install", detail: nil, tone: .idle, percent: nil, action: .install, busy: false)
        case .notReleased:
            self.init(label: "Not released yet", detail: "Not released yet, so there is nothing to install.", tone: .idle,
                      percent: nil, action: nil, busy: false)
        case .installing(let phase, let percent):
            let labels: [InstallProgress.Phase: String] = [
                .checking: "Checking", .downloading: "Downloading", .verifying: "Verifying",
                .installing: "Installing", .installed: "Installed", .failed: "Failed",
            ]
            self.init(label: labels[phase] ?? "Installing", detail: nil, tone: .live,
                      percent: phase == .downloading ? percent : nil, action: nil, busy: true)
        case .installed(let version, let available, _):
            if let available {
                self.init(label: "Update", detail: "v\(version) → \(available) available", tone: .blocked, percent: nil,
                          action: .update, busy: false)
            } else {
                self.init(label: "Installed", detail: "v\(version)", tone: .settled, percent: nil, action: nil, busy: false)
            }
        case .rejected(let error):
            self.init(label: "Couldn't install", detail: error, tone: .broken, percent: nil, action: .retry, busy: false)
        }
    }

    init(label: String, detail: String?, tone: StatusTone, percent: Double?, action: Action?, busy: Bool) {
        self.label = label
        self.detail = detail
        self.tone = tone
        self.percent = percent
        self.action = action
        self.busy = busy
    }

    /// canAddConnection: whether a listing in this state can take a connection now.
    public static func canAddConnection(_ state: PackState, source: ConnectorListing.Source, hasLegacyLaunch: Bool,
                                        kind: ConnectorKind) -> Bool {
        if kind == .extension { return false }
        if source == .builtin || source == .mcp { return true }
        if state.isInstalled { return true }
        if state == .notReleased { return false }
        return hasLegacyLaunch
    }
}

/// use-row-action.ts: what a connection row is doing, or why it could not.
public enum RowAction: String, Hashable, Sendable, CaseIterable {
    case backfill, delete, run, rollback, remove, signIn, signOut

    public var phrase: String {
        switch self {
        case .backfill: "Importing…"
        case .delete: "Removing…"
        case .run: "Polling…"
        case .rollback: "Rolling back…"
        case .remove: "Removing…"
        case .signIn: "Waiting for the sign-in window…"
        case .signOut: "Signing out…"
        }
    }

    public func key(_ id: String) -> String { "\(rawValue):\(id)" }
}

/// use-row-action.ts RowState.
public struct RowState: Hashable, Sendable {
    public var phrase: String?
    public var error: String?

    /// rowState: the first busy phrase wins over the first failure.
    public init(busy: [String: String], failed: [String: String], id: String, actions: [RowAction]) {
        phrase = actions.lazy.compactMap { busy[$0.key(id)] }.first
        error = phrase == nil ? actions.lazy.compactMap { failed[$0.key(id)] }.first : nil
    }
}
