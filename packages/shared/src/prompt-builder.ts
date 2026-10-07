import { TaskConfig, ProjectConfig } from './types'

/**
 * Builds a prompt specifically for review feedback (when sending inline
 * code review comments back to an agent). This is a lighter-weight prompt
 * that just provides the review context.
 */
export function buildFeedbackPrompt(
  feedback: string,
  task: TaskConfig,
  project: ProjectConfig
): string {
  const lines: string[] = []
  lines.push(`# Review Feedback for: ${task.title}`)
  lines.push('')
  lines.push(`**Project:** ${project.name}`)
  lines.push(`**Task ID:** ${task.id}`)
  lines.push('')
  lines.push(feedback)
  lines.push('')
  lines.push('Please address the review feedback above and update the task when done.')
  return lines.join('\n')
}
