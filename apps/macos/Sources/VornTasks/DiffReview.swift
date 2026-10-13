import Foundation
import VornCore

public enum DiffReview {
    /// buildFeedbackPrompt (prompt-builder.ts): the review wrapped for the agent that did the task.
    public static func prompt(_ feedback: String, task: VornTask, projectName: String) -> String {
        [
            "# Review Feedback for: \(task.title)", "",
            "**Project:** \(projectName)", "**Task ID:** \(task.id)", "",
            feedback, "",
            "Please address the review feedback above and update the task when done.",
        ].joined(separator: "\n")
    }
}
