import Foundation
import VornCore

/// workflow-files.ts: a workflow to and from a portable file.
extension WorkflowsStore {
    /// Export as file…: saves the draft as a file the user picks.
    public func exportWorkflow(_ workflow: WorkflowDefinition) {}

    /// The sidebar's import: reads a workflow file and adds it to the workspace.
    public func importWorkflow() {}
}
