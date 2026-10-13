import Foundation

/// onboarding-data.ts: one page of the welcome guide.
public struct OnboardingStep: Identifiable, Hashable, Sendable {
    public let id: String
    public let title: String
    public let subtitle: String
    /// The Lucide name the renderer draws.
    public let icon: String
}

/// onboarding-data.ts and App.tsx: the guide's pages and when it opens by itself.
public enum Onboarding {
    /// App.tsx `ONBOARDING_VERSION`: raised when the guide gains something worth a second look.
    public static let version = 2

    public static let steps: [OnboardingStep] = [
        OnboardingStep(id: "agents", title: "Coding Agents", subtitle: "Detect agents & install skills", icon: "Bot"),
        OnboardingStep(id: "projects", title: "Projects & Tasks", subtitle: "Organize repos, branches & work items",
                       icon: "FolderGit2"),
        OnboardingStep(id: "sessions", title: "Launch Sessions", subtitle: "Start agents with the right context",
                       icon: "Rocket"),
        OnboardingStep(id: "workspace", title: "Your Workspace", subtitle: "Grid, tabs & the review panel",
                       icon: "LayoutDashboard"),
        OnboardingStep(id: "workflows", title: "Workflows & Schedules", subtitle: "Automate multi-agent pipelines",
                       icon: "Workflow"),
        OnboardingStep(id: "shortcuts", title: "Command Palette & Shortcuts", subtitle: "Find anything instantly",
                       icon: "Search"),
        OnboardingStep(id: "ready", title: "You're Ready", subtitle: "Start building with your agents", icon: "Sparkles"),
    ]

    /// App.tsx: opens when `Number(defaults.hasSeenOnboarding ?? 0) < ONBOARDING_VERSION`; a value
    /// that is not a number (NaN) never compares less, so it keeps the guide shut.
    public static func shouldOpen(hasSeen: JSONValue?) -> Bool {
        guard let n = jsNumber(hasSeen ?? .number(0)) else { return false }
        return n < Double(version)
    }

    /// The step after `step`, held at the last (ArrowRight).
    public static func next(_ step: Int) -> Int { min(steps.count - 1, step + 1) }
    /// The step before `step`, held at the first (ArrowLeft).
    public static func previous(_ step: Int) -> Int { max(0, step - 1) }
    /// `step` brought into range, for a step named from outside.
    public static func clamp(_ step: Int) -> Int { max(0, min(steps.count - 1, step)) }

    /// OnboardingModal.tsx: the agents whose plugin setup the first page lists, those installed or else the first two.
    public static func mcpAgents(order: [AgentType], installed: (AgentType) -> Bool) -> [AgentType] {
        let found = order.filter(installed)
        return found.isEmpty ? Array(order.prefix(2)) : found
    }

    /// JavaScript's `Number(value)`, with nil for NaN.
    static func jsNumber(_ value: JSONValue) -> Double? {
        switch value {
        case .null: return 0
        case .bool(let b): return b ? 1 : 0
        case .number(let n): return n.isNaN ? nil : n
        case .string(let s):
            let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
            if t.isEmpty { return 0 }
            if t == "Infinity" || t == "+Infinity" { return .infinity }
            if t == "-Infinity" { return -.infinity }
            let lower = t.lowercased()
            for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] where lower.hasPrefix(prefix) {
                return UInt64(lower.dropFirst(2), radix: radix).map(Double.init)
            }
            guard t.allSatisfy({ "0123456789.eE+-".contains($0) }), let d = Double(t), d.isFinite else { return nil }
            return d
        case .array(let a):
            if a.isEmpty { return 0 }
            return a.count == 1 ? jsString(a[0]).flatMap { jsNumber(.string($0)) } : nil
        case .object: return nil
        }
    }

    /// `String(value)` for what a one-element array turns into before `Number()` reads it.
    private static func jsString(_ value: JSONValue) -> String? {
        switch value {
        case .null: return ""
        case .bool(let b): return b ? "true" : "false"
        case .number(let n): return n == n.rounded() && abs(n) < 1e21 ? String(Int64(n)) : String(n)
        case .string(let s): return s
        case .array(let a): return a.count <= 1 ? (a.first.flatMap(jsString) ?? "") : nil
        case .object: return nil
        }
    }
}
