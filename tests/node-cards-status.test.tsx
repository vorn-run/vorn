// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockConfig = { remoteHosts: [] as Array<{ id: string; label: string }> }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = { config: mockConfig }
    return selector ? selector(state) : state
  }
}))

vi.mock('../src/renderer/components/AgentIcon', () => ({
  AgentIcon: () => <div data-testid="agent-icon" />
}))

const { TriggerNode } = await import('../src/renderer/components/workflow-editor/nodes/TriggerNode')
const { ScriptNode } = await import('../src/renderer/components/workflow-editor/nodes/ScriptNode')
const { ConditionNode } =
  await import('../src/renderer/components/workflow-editor/nodes/ConditionNode')
const { LaunchAgentNode } =
  await import('../src/renderer/components/workflow-editor/nodes/LaunchAgentNode')

describe('node cards — executionStatus dot', () => {
  it('renders a success dot on ScriptNode', () => {
    const { container } = render(
      <ScriptNode
        label="Script"
        config={{ scriptType: 'bash', scriptContent: '' }}
        executionStatus="success"
        onClick={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.success}`)).toBeInTheDocument()
  })

  it('renders an error dot on ConditionNode', () => {
    const { container } = render(
      <ConditionNode
        label="Cond"
        config={{ variable: 'x', operator: 'equals', value: '1' }}
        executionStatus="error"
        onClick={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.error}`)).toBeInTheDocument()
  })

  it('renders a running, pulsing dot on LaunchAgentNode', () => {
    const { container } = render(
      <LaunchAgentNode
        label="Agent"
        config={{ agentType: 'claude', projectName: 'x', projectPath: '/x' }}
        executionStatus="running"
        onClick={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.running}`)).toBeInTheDocument()
    expect(container.querySelector('.animate-pulse')).toBeInTheDocument()
  })

  it('renders no status dot on TriggerNode (trigger has no execution status)', () => {
    const { container } = render(
      <TriggerNode label="T" config={{ triggerType: 'manual' }} onClick={vi.fn()} />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.success}`)).not.toBeInTheDocument()
  })
})

describe('node cards — running breath and failure border', () => {
  it('breathes a border overlay while running', () => {
    const { container } = render(
      <ScriptNode
        label="Script"
        config={{ scriptType: 'bash', scriptContent: '' }}
        selected={false}
        onClick={() => {}}
        executionStatus="running"
      />
    )
    expect(container.querySelector('span.absolute.inset-0.animate-pulse')).toBeInTheDocument()
  })

  it('borders danger on the failed step, and only there', () => {
    const failed = render(
      <ScriptNode
        label="Script"
        config={{ scriptType: 'bash', scriptContent: '' }}
        selected={false}
        onClick={() => {}}
        executionStatus="error"
      />
    )
    expect((failed.container.firstChild as HTMLElement).className).toContain('border-danger')
    expect(failed.container.querySelector('span.absolute.inset-0.animate-pulse')).toBeNull()

    const idle = render(
      <ScriptNode
        label="Script"
        config={{ scriptType: 'bash', scriptContent: '' }}
        selected={false}
        onClick={() => {}}
      />
    )
    expect((idle.container.firstChild as HTMLElement).className).not.toContain('border-danger')
  })
})
