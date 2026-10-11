import Foundation
import Observation
import VornCore

/// The gate a review page is open for.
public struct GateReviewTarget: Equatable, Sendable {
    public var runId: String
    public var nodeId: String

    public init(runId: String, nodeId: String) {
        self.runId = runId
        self.nodeId = nodeId
    }
}

/// One schedule that came due while Vorn was closed (MissedScheduleDialog.tsx MissedItem).
public struct MissedSchedule: Equatable, Sendable, Identifiable {
    public var workflowId: String
    public var name: String
    public var scheduledFor: String

    public var id: String { workflowId }

    public init(workflowId: String, name: String, scheduledFor: String) {
        self.workflowId = workflowId
        self.name = name
        self.scheduledFor = scheduledFor
    }

    /// Reads the preload's `[{workflow:{id,name}, scheduledFor}]` and protocol.ts's `[{workflowId, workflowName, missedAt}]`.
    public static func list(_ json: JSONValue) -> [MissedSchedule] {
        (json.arrayValue ?? []).compactMap { item in
            if let wf = item["workflow"], let id = wf["id"]?.stringValue {
                return MissedSchedule(
                    workflowId: id, name: wf["name"]?.stringValue ?? id, scheduledFor: item["scheduledFor"]?.stringValue ?? "")
            }
            guard let id = item["workflowId"]?.stringValue else { return nil }
            return MissedSchedule(
                workflowId: id, name: item["workflowName"]?.stringValue ?? id,
                scheduledFor: item["missedAt"]?.stringValue ?? item["scheduledFor"]?.stringValue ?? "")
        }
    }

    /// "Scheduled for <locale date>", as `toLocaleString()` words it.
    public var scheduledLabel: String {
        guard let date = TimeFormat.parse(scheduledFor) else { return "Scheduled for \(scheduledFor)" }
        return "Scheduled for \(date.formatted(date: .numeric, time: .standard))"
    }
}

/// The run surfaces drawn over the window: the run prompt, full output, a gate's review page, missed schedules.
@Observable @MainActor
public final class RunSurfaces {
    public var prompt: RunPrompt?
    /// The logs View full output shows.
    public var fullOutput: String?
    public var review: GateReviewTarget?
    public var missed: [MissedSchedule] = []
    public var missedSelected: Set<String> = []
    /// Connection names by id, for "Sign in to <name>".
    public var connectionNames: [String: String] = [:]
    @ObservationIgnored var connectionsLoaded = false

    public init() {}

    /// A modal is open, so the run detail's keys stand down.
    public var hasModal: Bool { prompt != nil || fullOutput != nil || review != nil || !missed.isEmpty }
}

/// The pending manual run and what its prompt holds (SourcePromptDialog.tsx).
@Observable @MainActor
public final class RunPrompt: Identifiable {
    public let workflow: WorkflowDefinition
    public let context: ManualRunContext?
    public let targetNodeId: String?
    public let defs: [WorkflowInputDef]
    public let contextual: Bool
    public let required: RunInputs.RequiredFields
    public let projects: [Project]
    public var projectName: String
    public var projectPath: String
    public var branch = ""
    public var useWorktree = false
    public var values: [String: JSONValue]

    public nonisolated var id: String { workflow.id }

    public init(workflow: WorkflowDefinition, context: ManualRunContext?, targetNodeId: String?, projects: [Project]) {
        self.workflow = workflow
        self.context = context
        self.targetNodeId = targetNodeId
        self.projects = projects
        defs = RunInputs.inputs(workflow)
        contextual = RunInputs.isContextual(workflow)
        if contextual && context?.hasSource != true {
            var r = RunInputs.requiredFields(workflow)
            r.needsProject = true
            required = r
        } else {
            required = RunInputs.RequiredFields()
        }
        projectName = projects.first?.name ?? ""
        projectPath = projects.first?.path ?? ""
        values = RunInputs.initialValues(defs)
    }

    public var hasCallerContext: Bool { context?.hasSource == true }

    public var canSubmit: Bool {
        !(required.needsProject && (projectName.isEmpty || projectPath.isEmpty)) && RunInputs.areValid(defs, values)
    }

    public func selectProject(_ name: String) {
        guard let p = projects.first(where: { $0.name == name }) else { return }
        projectName = p.name
        projectPath = p.path
    }

    /// The `workflow:run` params: the caller's context or a source made from the fields, plus the inputs.
    public func params(now: Date = Date()) -> JSONValue {
        var ctx: [String: JSONValue] = [:]
        if hasCallerContext, let context {
            ctx = context.json
        } else if contextual {
            let ms = (now.timeIntervalSince1970 * 1000).rounded()
            var source: [String: JSONValue] = [
                "id": .string("prompt:\(workflow.id):\(Int64(ms))"), "agentType": "shell",
                "projectName": .string(projectName), "projectPath": .string(projectPath), "status": "idle",
                "createdAt": .number(ms), "pid": .number(0), "isWorktree": .bool(useWorktree),
            ]
            if !branch.isEmpty { source["branch"] = .string(branch) }
            ctx["source"] = .object(source)
        }
        if !defs.isEmpty { ctx["inputs"] = .object(values) }
        var params: [String: JSONValue] = ["workflowId": .string(workflow.id), "context": .object(ctx)]
        if let targetNodeId { params["targetNodeId"] = .string(targetNodeId) }
        return .object(params)
    }
}
