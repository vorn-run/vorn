// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

import { NodeCard } from '../src/renderer/components/workflow-editor/nodes/NodeCard'
import type { WorkflowNode } from '../packages/shared/src/types'

afterEach(cleanup)

const card = (type: WorkflowNode['type'], label: string, config: Record<string, unknown>) =>
  render(
    <NodeCard
      node={{
        id: 'n',
        type,
        label,
        config: config as WorkflowNode['config'],
        position: { x: 0, y: 0 }
      }}
      selected={false}
      onClick={() => {}}
    />
  )

describe('the card for each step type', () => {
  it.each([
    ['trigger', 'When it starts', { triggerType: 'manual' }],
    ['script', 'Run checks', { scriptType: 'bash', scriptContent: '' }],
    ['condition', 'Ready?', { variable: 'x', operator: 'equals', value: '1' }],
    ['approval', 'Sign off', { message: '' }],
    ['createTaskFromItem', 'File it', { project: 'vorn' }],
    ['callConnectorAction', 'Comment on issue', { connectionId: '', action: '', args: {} }],
    ['launchAgent', 'Do the work', { agentType: 'claude', projectName: '', projectPath: '' }]
  ] as const)('renders a %s card by its label', (type, label, config) => {
    card(type, label, config as Record<string, unknown>)
    expect(screen.getByText(label)).toBeInTheDocument()
  })
})
