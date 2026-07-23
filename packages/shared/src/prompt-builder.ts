import { TaskConfig, ProjectConfig, TaskStatus, WorkflowDefinition } from './types'
import { buildStructuredOutputInstructions } from './structured-output'

export interface TaskPromptContext {
  task: TaskConfig
  project: ProjectConfig
  /** Other tasks in the same project (for cross-task awareness) */
  siblingTasks?: TaskConfig[]
  /** Current git branch (if known at launch time) */
  currentBranch?: string
}

export interface WorkflowPromptContext {
  workflow: WorkflowDefinition
  stepName: string
  userPrompt: string
  /** When set, the agent is asked to end its run with a JSON object matching
   *  this schema so the engine can capture typed step output. */
  outputSchema?: Record<string, unknown>
}

/**
 * Builds a structured prompt that gives an AI agent full context about
 * what it's working on. This replaces passing raw `task.description`.
 *
 * The prompt includes:
 * - Task title, status, and description
 * - Project name and path
 * - Branch info
 * - Summary of related tasks in the project
 * - Instructions to use MCP tools for more context
 */
export function buildTaskPrompt(ctx: TaskPromptContext): string {
  const { task, project, siblingTasks, currentBranch } = ctx
  const lines: string[] = []

  // --- Task header ---
  lines.push(`# Task: ${task.title}`)
  lines.push('')

  // --- Project + branch context ---
  lines.push(`**Project:** ${project.name}`)
  lines.push(`**Project Path:** ${project.path}`)
  const branch = task.branch || currentBranch
  if (branch) {
    lines.push(`**Branch:** ${branch}`)
  }
  if (task.useWorktree) {
    lines.push(`**Worktree:** Yes (isolated git worktree)`)
  }
  lines.push(`**Task Status:** ${formatStatus(task.status)}`)
  lines.push(`**Task ID:** ${task.id}`)
  lines.push('')

  // --- Task description (the actual work to do) ---
  if (task.description && task.description.trim()) {
    lines.push('## Description')
    lines.push('')
    lines.push(task.description.trim())
    lines.push('')
  }

  // --- Sibling tasks summary (cross-task awareness) ---
  if (siblingTasks && siblingTasks.length > 0) {
    const others = siblingTasks.filter((t) => t.id !== task.id)
    const inProgress = others.filter((t) => t.status === 'in_progress')
    const inReview = others.filter((t) => t.status === 'in_review')
    const todo = others.filter((t) => t.status === 'todo')

    if (inProgress.length > 0 || inReview.length > 0 || todo.length > 0) {
      lines.push('## Other Tasks in This Project')
      lines.push('')
      if (inProgress.length > 0) {
        lines.push(`**In Progress (${inProgress.length}):**`)
        for (const t of inProgress.slice(0, 5)) {
          lines.push(`- ${t.title}${t.branch ? ` (branch: ${t.branch})` : ''}`)
        }
        if (inProgress.length > 5) lines.push(`- ... and ${inProgress.length - 5} more`)
        lines.push('')
      }
      if (inReview.length > 0) {
        lines.push(`**In Review (${inReview.length}):**`)
        for (const t of inReview.slice(0, 5)) {
          lines.push(`- ${t.title}${t.branch ? ` (branch: ${t.branch})` : ''}`)
        }
        if (inReview.length > 5) lines.push(`- ... and ${inReview.length - 5} more`)
        lines.push('')
      }
      if (todo.length > 0) {
        lines.push(`**Queued (${todo.length}):**`)
        for (const t of todo.slice(0, 3)) {
          lines.push(`- ${t.title}`)
        }
        if (todo.length > 3) lines.push(`- ... and ${todo.length - 3} more`)
        lines.push('')
      }
    }
  }

  // --- MCP instructions ---
  lines.push('## Available Tools')
  lines.push('')
  lines.push('You are managed by Vorn. You have access to MCP tools for project management:')
  lines.push('- `get_my_context` — Get your current task and project context')
  lines.push('- `list_tasks` — List tasks in this project')
  lines.push('- `update_task` — Update task status/description when done')
  lines.push('- `get_diff` — See current git changes')
  lines.push('- `list_branches` — List git branches')
  lines.push('')
  lines.push(
    'When you complete this task, update its status to "in_review" or "done" using `update_task`.'
  )
  lines.push('')

  return lines.join('\n')
}

/**
 * Wraps a user prompt with workflow context so the agent knows which
 * workflow and step it belongs to, and can use MCP tools to check
 * previous run logs.
 */
export function buildWorkflowPrompt(ctx: WorkflowPromptContext): string {
  const { workflow, stepName, userPrompt, outputSchema } = ctx
  const lines: string[] = []

  lines.push(`# Workflow: ${workflow.name}`)
  lines.push('')
  lines.push(`**Step:** ${stepName}`)
  lines.push(`**Workflow ID:** ${workflow.id}`)
  lines.push('')

  lines.push('## Task')
  lines.push('')
  lines.push(userPrompt)
  lines.push('')

  lines.push('## Available Tools')
  lines.push('')
  lines.push('You are managed by Vorn. You have access to MCP tools:')
  lines.push('- `get_my_context` — Get your current project context')
  lines.push(
    `- \`list_workflow_runs\` — See previous runs for this workflow (workflow_id: "${workflow.id}")`
  )
  lines.push('- `list_tasks` — List tasks in this project')
  lines.push('- `update_task` — Update task status when done')
  lines.push('')

  if (outputSchema) {
    lines.push(buildStructuredOutputInstructions(outputSchema))
  }

  return lines.join('\n')
}

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

function formatStatus(status: TaskStatus): string {
  switch (status) {
    case 'todo':
      return 'To Do'
    case 'in_progress':
      return 'In Progress'
    case 'in_review':
      return 'In Review'
    case 'done':
      return 'Done'
    case 'cancelled':
      return 'Cancelled'
    default:
      return status
  }
}
