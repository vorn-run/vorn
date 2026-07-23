import { describe, it, expect } from 'vitest'
import {
  buildTaskPrompt,
  buildFeedbackPrompt,
  buildWorkflowPrompt
} from '@vornrun/shared/prompt-builder'
import type { TaskConfig, ProjectConfig, WorkflowDefinition } from '@vornrun/shared/types'

function makeTask(overrides: Partial<TaskConfig> = {}): TaskConfig {
  return {
    id: 'task-001',
    projectName: 'vorn',
    title: 'Fix login bug',
    description: 'Users cannot log in on Safari',
    status: 'in_progress',
    order: 0,
    createdAt: '2025-01-01T00:00:00Z',
    updatedAt: '2025-01-01T00:00:00Z',
    ...overrides
  }
}

function makeProject(overrides: Partial<ProjectConfig> = {}): ProjectConfig {
  return {
    name: 'vorn',
    path: '/Users/dev/vorn',
    preferredAgents: ['claude'],
    ...overrides
  }
}

describe('buildTaskPrompt', () => {
  it('includes task title', () => {
    const result = buildTaskPrompt({ task: makeTask(), project: makeProject() })
    expect(result).toContain('# Task: Fix login bug')
  })

  it('includes project name and path', () => {
    const result = buildTaskPrompt({ task: makeTask(), project: makeProject() })
    expect(result).toContain('**Project:** vorn')
    expect(result).toContain('**Project Path:** /Users/dev/vorn')
  })

  it('includes branch when present on task', () => {
    const result = buildTaskPrompt({
      task: makeTask({ branch: 'fix/login' }),
      project: makeProject()
    })
    expect(result).toContain('**Branch:** fix/login')
  })

  it('uses currentBranch fallback when task branch is absent', () => {
    const result = buildTaskPrompt({
      task: makeTask(),
      project: makeProject(),
      currentBranch: 'main'
    })
    expect(result).toContain('**Branch:** main')
  })

  it('includes worktree note when useWorktree is true', () => {
    const result = buildTaskPrompt({
      task: makeTask({ useWorktree: true }),
      project: makeProject()
    })
    expect(result).toContain('**Worktree:** Yes')
  })

  it('includes description section', () => {
    const result = buildTaskPrompt({ task: makeTask(), project: makeProject() })
    expect(result).toContain('## Description')
    expect(result).toContain('Users cannot log in on Safari')
  })

  it('omits description section when empty', () => {
    const result = buildTaskPrompt({
      task: makeTask({ description: '' }),
      project: makeProject()
    })
    expect(result).not.toContain('## Description')
  })

  it('includes sibling tasks grouped by status', () => {
    const siblings = [
      makeTask({ id: 'other-1', title: 'Other task', status: 'in_progress' }),
      makeTask({ id: 'other-2', title: 'Review task', status: 'in_review' }),
      makeTask({ id: 'other-3', title: 'Todo task', status: 'todo' })
    ]
    const result = buildTaskPrompt({
      task: makeTask(),
      project: makeProject(),
      siblingTasks: siblings
    })
    expect(result).toContain('## Other Tasks in This Project')
    expect(result).toContain('**In Progress (1):**')
    expect(result).toContain('**In Review (1):**')
    expect(result).toContain('**Queued (1):**')
  })

  it('truncates in_progress at 5 and todo at 3', () => {
    const inProgress = Array.from({ length: 7 }, (_, i) =>
      makeTask({ id: `ip-${i}`, title: `IP ${i}`, status: 'in_progress' })
    )
    const todos = Array.from({ length: 5 }, (_, i) =>
      makeTask({ id: `todo-${i}`, title: `Todo ${i}`, status: 'todo' })
    )
    const result = buildTaskPrompt({
      task: makeTask(),
      project: makeProject(),
      siblingTasks: [...inProgress, ...todos]
    })
    expect(result).toContain('... and 2 more')
    expect(result).toContain('... and 2 more')
  })

  it('includes MCP tool instructions', () => {
    const result = buildTaskPrompt({ task: makeTask(), project: makeProject() })
    expect(result).toContain('get_my_context')
    expect(result).toContain('update_task')
  })
})

describe('buildFeedbackPrompt', () => {
  it('includes task title and project', () => {
    const result = buildFeedbackPrompt('Please fix tests', makeTask(), makeProject())
    expect(result).toContain('# Review Feedback for: Fix login bug')
    expect(result).toContain('**Project:** vorn')
  })

  it('includes feedback text', () => {
    const result = buildFeedbackPrompt('Tests are failing', makeTask(), makeProject())
    expect(result).toContain('Tests are failing')
  })

  it('includes task ID', () => {
    const result = buildFeedbackPrompt('Fix it', makeTask(), makeProject())
    expect(result).toContain('**Task ID:** task-001')
  })
})

describe('buildWorkflowPrompt', () => {
  const workflow: WorkflowDefinition = {
    id: 'wf-123',
    name: 'Deploy Pipeline',
    icon: 'Rocket',
    iconColor: '#ef4444',
    nodes: [],
    edges: [],
    enabled: true
  }

  it('includes workflow name and step', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Run tests',
      userPrompt: 'Execute the test suite'
    })
    expect(result).toContain('# Workflow: Deploy Pipeline')
    expect(result).toContain('**Step:** Run tests')
  })

  it('includes workflow ID', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Build',
      userPrompt: 'Build the project'
    })
    expect(result).toContain('**Workflow ID:** wf-123')
  })

  it('includes the user prompt under Task section', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Deploy',
      userPrompt: 'Deploy to staging'
    })
    expect(result).toContain('## Task')
    expect(result).toContain('Deploy to staging')
  })

  it('includes MCP tools with workflow_id for list_workflow_runs', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Check',
      userPrompt: 'Check status'
    })
    expect(result).toContain('list_workflow_runs')
    expect(result).toContain('wf-123')
    expect(result).toContain('get_my_context')
  })

  it('appends the structured-output instructions when an outputSchema is given', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Review',
      userPrompt: 'Review the PR',
      outputSchema: {
        type: 'object',
        properties: { verdict: { type: 'string' } },
        required: ['verdict']
      }
    })
    expect(result).toContain('## Required Output')
    expect(result).toContain('<<<VORN_OUTPUT>>>')
    expect(result).toContain('"verdict"')
  })

  it('omits the structured-output section when no schema is given', () => {
    const result = buildWorkflowPrompt({
      workflow,
      stepName: 'Review',
      userPrompt: 'Review the PR'
    })
    expect(result).not.toContain('Required Output')
    expect(result).not.toContain('<<<VORN_OUTPUT>>>')
  })
})
