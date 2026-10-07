import { describe, it, expect } from 'vitest'
import { buildFeedbackPrompt } from '@vornrun/shared/prompt-builder'
import type { TaskConfig, ProjectConfig } from '@vornrun/shared/types'

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
