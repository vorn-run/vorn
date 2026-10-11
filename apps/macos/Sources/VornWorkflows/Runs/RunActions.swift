import Foundation
import VornCore

/// workflow-menu-items.tsx startManualRun, SourcePromptDialog submit, and MissedScheduleDialog's buttons.
extension WorkflowsStore {
    /// Runs now, or opens the run prompt first when the trigger asks for inputs or a source.
    func startManualRun(_ workflow: WorkflowDefinition, context: ManualRunContext?, targetNodeId: String?) {
        guard workflow.triggerNode != nil else {
            toast = "\"\(workflow.name)\" has no trigger — add one in the editor first"
            return
        }
        if RunInputs.needsPrompt(workflow, context: context) {
            surfaces.prompt = RunPrompt(
                workflow: workflow, context: context, targetNodeId: targetNodeId, projects: projects)
            return
        }
        var params: [String: JSONValue] = ["workflowId": .string(workflow.id)]
        if let context, context.hasSource { params["context"] = .object(context.json) }
        if let targetNodeId { params["targetNodeId"] = .string(targetNodeId) }
        launch(.object(params))
    }

    /// Run: starts the prompt's run and closes it.
    public func submitPrompt() {
        guard let prompt = surfaces.prompt, prompt.canSubmit else { return }
        surfaces.prompt = nil
        launch(prompt.params())
    }

    public func cancelPrompt() { surfaces.prompt = nil }

    private func launch(_ params: JSONValue) {
        perform { [backend] in
            let raw = try await backend.rpc("workflow:run", params)
            if let run = try? raw.decode(WorkflowExecution.self) { self.upsert(run) }
        }
    }

    // MARK: Missed schedules

    /// A `scheduler:missed` push: every item starts selected.
    func showMissed(_ items: [MissedSchedule]) {
        guard !items.isEmpty else { return }
        surfaces.missed = items
        surfaces.missedSelected = Set(items.map(\.workflowId))
    }

    public func toggleMissed(_ id: String) {
        if surfaces.missedSelected.contains(id) {
            surfaces.missedSelected.remove(id)
        } else {
            surfaces.missedSelected.insert(id)
        }
    }

    /// Run Selected: each still-existing selected workflow starts, none waited on.
    public func runMissedSelected() {
        let ids = surfaces.missed.map(\.workflowId).filter { surfaces.missedSelected.contains($0) && workflow($0) != nil }
        surfaces.missed = []
        for id in ids { launch(.object(["workflowId": .string(id)])) }
    }

    public func skipMissed() { surfaces.missed = [] }

    // MARK: Full output and sign-in

    public func showFullOutput(_ logs: String) { surfaces.fullOutput = logs }

    /// SignInButton.tsx: the sign-in window is browser-hosted and not native yet.
    public func signIn(connectionId: String) {
        toast = "Sign in from the Vorn desktop app."
    }

    /// Names for "Sign in to <connection>", read once.
    public func loadConnectionNames() async {
        guard !surfaces.connectionsLoaded else { return }
        surfaces.connectionsLoaded = true
        guard let list = try? await backend.rpc("connection:list", nil).arrayValue else { return }
        for c in list {
            if let id = c["id"]?.stringValue { surfaces.connectionNames[id] = c["name"]?.stringValue ?? id }
        }
    }
}
