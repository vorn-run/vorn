import Foundation
import VornCore

/// What a run update is worth announcing (run-notifications.ts): a gate or sign-in
/// that just started waiting, and a run that just ended. Remembers what it saw, as
/// every update carries the whole run.
public struct RunAnnouncer: Sendable {
    public enum Announcement: Sendable, Equatable {
        /// A step started waiting on the user; shown under the waiting preference.
        case wait(AppNotice)
        /// The run ended after this window saw it running.
        case ended(runId: String, body: String)
    }

    private struct Seen: Sendable {
        var status: RunStatus
        var waiting: Set<String>
    }

    private var lastSeen: [String: Seen] = [:]

    public init() {}

    public mutating func announce(_ run: WorkflowExecution, workflow: WorkflowDefinition?) -> [Announcement] {
        let previous = lastSeen[run.runId]
        let waiting = Set(run.nodeStates.filter { $0.status == .waiting }.map(\.nodeId))
        var out: [Announcement] = []

        if let workflow {
            for state in run.nodeStates where state.status == .waiting && previous?.waiting.contains(state.nodeId) != true {
                let node = workflow.nodes.first { $0.id == state.nodeId }
                if state.isSignInWait {
                    out.append(.wait(AppNotice(
                        reason: .waiting, key: "workflow-sign-in:\(workflow.id):\(state.nodeId)",
                        title: "\(workflow.name) · awaiting sign-in",
                        body: nonEmpty(state.error) ?? "\(node?.label ?? "A step") is waiting for you to sign in again",
                        target: .run(run.runId))))
                } else {
                    let message = state.message ?? node?.config["message"]?.stringValue
                    out.append(.wait(AppNotice(
                        reason: .waiting, key: "workflow-gate:\(workflow.id):\(state.nodeId)",
                        title: "\(workflow.name) · awaiting approval",
                        body: nonEmpty(message) ?? "\(node?.label ?? "Approval") is waiting for your approval",
                        target: .run(run.runId))))
                }
            }
        }

        let ended = run.status == .success || run.status == .error
        if ended, previous?.status == .running, let workflow {
            let steps = run.nodeStates.filter { $0.status != .pending }.count
            let outcome = run.status == .success ? "completed" : RunPresenter.isRejected(run) ? "was rejected" : "failed"
            out.append(.ended(runId: run.runId,
                              body: "Workflow \"\(workflow.name)\" \(outcome) — \(steps) step\(steps == 1 ? "" : "s")"))
        }

        // A stopped run gets no further update, so nothing is kept for a run that is over.
        if run.status != .running {
            lastSeen[run.runId] = nil
        } else {
            lastSeen[run.runId] = Seen(status: run.status, waiting: waiting)
        }
        return out
    }

    private func nonEmpty(_ s: String?) -> String? {
        guard let s, !s.isEmpty else { return nil }
        return s
    }
}
