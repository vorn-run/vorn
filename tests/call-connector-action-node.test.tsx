// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { CallConnectorActionConfig, WorkflowNode } from '../src/shared/types'
import { CallConnectorActionNode } from '../src/renderer/components/workflow-editor/nodes/CallConnectorActionNode'
import { stepPreview } from '../src/renderer/components/workflow-editor/node-visuals'
import { estimateNodeHeight } from '../src/renderer/lib/workflow-canvas-layout'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

const config = (over: Partial<CallConnectorActionConfig> = {}): CallConnectorActionConfig =>
  ({
    nodeType: 'callConnectorAction',
    connectionId: 'c1',
    action: 'echo',
    ...over
  }) as CallConnectorActionConfig

const nodeFor = (cfg: CallConnectorActionConfig): WorkflowNode => ({
  id: 'n',
  type: 'callConnectorAction',
  label: 'Echo',
  config: cfg,
  position: { x: 0, y: 0 }
})

describe('CallConnectorActionNode', () => {
  it('draws no footer, and is estimated as a bare card, without arguments', () => {
    const bare = config()
    const { container } = render(
      <CallConnectorActionNode label="Echo" config={bare} onClick={vi.fn()} />
    )
    expect(container.querySelector('.border-t')).toBeNull()
    expect(stepPreview(nodeFor(bare))).toBeUndefined()
    expect(estimateNodeHeight(nodeFor(bare), [])).toBe(58)
  })

  it('draws the arguments as a footer exactly when the estimate charges for one', () => {
    const withArgs = config({ args: { message: 'hi' } })
    const { container } = render(
      <CallConnectorActionNode label="Echo" config={withArgs} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('message: hi')
    expect(container.querySelector('.border-t')).not.toBeNull()
    expect(stepPreview(nodeFor(withArgs))).toBe('message: hi')
    expect(estimateNodeHeight(nodeFor(withArgs), [])).toBe(90)
  })
})
