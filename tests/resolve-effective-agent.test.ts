// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'

// The resolver reads defaults.defaultAgent as the penultimate fallback, from
// the server's configuration. Mock the host before importing the subject so
// the value is stable across imports.
const mockState = {
  config: {
    defaults: { defaultAgent: undefined as string | undefined }
  }
}

vi.mock('../packages/server/src/workflows/host', () => ({
  config: () => mockState.config,
  api: {},
  publishRun: vi.fn(),
  runById: vi.fn(),
  activeTerminals: () => [],
  activeHeadless: () => [],
  nextTask: () => undefined,
  onHeadlessData: () => () => {},
  onHeadlessExit: () => () => {},
  onScriptData: () => () => {}
}))

// Subject under test must import after the mock is registered.
const { resolveEffectiveAgent } = await import('../packages/server/src/workflows/engine')

import type {
  AiAgentType,
  LaunchAgentConfig,
  TaskConfig,
  WorkflowExecutionContext
} from '../src/shared/types'

function makeConfig(agentType: LaunchAgentConfig['agentType']): LaunchAgentConfig {
  return {
    agentType,
    projectName: 'p',
    projectPath: '/p'
  }
}

function makeTask(assignedAgent: AiAgentType | undefined): TaskConfig {
  return {
    id: 't1',
    projectName: 'p',
    title: 'test',
    description: '',
    status: 'in_progress',
    order: 0,
    assignedAgent,
    createdAt: '2026-04-15T00:00:00Z',
    updatedAt: '2026-04-15T00:00:00Z'
  }
}

describe('resolveEffectiveAgent', () => {
  beforeEach(() => {
    mockState.config.defaults.defaultAgent = undefined
  })

  it('returns the concrete agentType unchanged when it is not "fromTask"', () => {
    expect(resolveEffectiveAgent(makeConfig('claude'), undefined, undefined)).toBe('claude')
    expect(resolveEffectiveAgent(makeConfig('codex'), undefined, undefined)).toBe('codex')
  })

  it('ignores context and resolved task when agentType is concrete', () => {
    const ctx: WorkflowExecutionContext = { task: makeTask('codex') }
    expect(resolveEffectiveAgent(makeConfig('gemini'), ctx, makeTask('copilot'))).toBe('gemini')
  })

  it('reads context.task.assignedAgent first when agentType is "fromTask"', () => {
    const ctx: WorkflowExecutionContext = { task: makeTask('codex') }
    expect(resolveEffectiveAgent(makeConfig('fromTask'), ctx, makeTask('copilot'))).toBe('codex')
  })

  it('falls back to resolvedTask.assignedAgent when context task has no agent', () => {
    const ctx: WorkflowExecutionContext = { task: makeTask(undefined) }
    expect(resolveEffectiveAgent(makeConfig('fromTask'), ctx, makeTask('gemini'))).toBe('gemini')
  })

  it('uses resolvedTask.assignedAgent when there is no context at all', () => {
    expect(resolveEffectiveAgent(makeConfig('fromTask'), undefined, makeTask('opencode'))).toBe(
      'opencode'
    )
  })

  it('falls back to defaults.defaultAgent when no task has an assignment', () => {
    mockState.config.defaults.defaultAgent = 'copilot'
    expect(resolveEffectiveAgent(makeConfig('fromTask'), undefined, makeTask(undefined))).toBe(
      'copilot'
    )
  })

  it('falls back to claude as the final default', () => {
    expect(resolveEffectiveAgent(makeConfig('fromTask'), undefined, undefined)).toBe('claude')
  })
})
