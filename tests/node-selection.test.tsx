// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = { connections: [], workflowExecutions: new Map() }
    return selector ? selector(state) : state
  }
}))

import { TriggerNode } from '../src/renderer/components/workflow-editor/nodes/TriggerNode'
import { LaunchAgentNode } from '../src/renderer/components/workflow-editor/nodes/LaunchAgentNode'
import { ScriptNode } from '../src/renderer/components/workflow-editor/nodes/ScriptNode'
import { ConditionNode } from '../src/renderer/components/workflow-editor/nodes/ConditionNode'
import { ApprovalNode } from '../src/renderer/components/workflow-editor/nodes/ApprovalNode'
import { LoopNode } from '../src/renderer/components/workflow-editor/nodes/LoopNode'
import { CreateTaskFromItemNode } from '../src/renderer/components/workflow-editor/nodes/CreateTaskFromItemNode'
import { CallConnectorActionNode } from '../src/renderer/components/workflow-editor/nodes/CallConnectorActionNode'
import {
  NODE_SELECTED,
  NODE_UNSELECTED
} from '../src/renderer/components/workflow-editor/node-visuals'

/**
 * Every card that can be selected, rendered both ways.
 *
 * The selection string used to be pasted into each of these, and two had
 * already drifted away from the rest. Checking one card could not have caught
 * that, so this checks all of them against the one constant.
 */
const CARDS: [string, (selected: boolean) => React.ReactElement][] = [
  [
    'TriggerNode',
    (s) => (
      <TriggerNode label="T" config={{ triggerType: 'manual' }} selected={s} onClick={vi.fn()} />
    )
  ],
  [
    'LaunchAgentNode',
    (s) => (
      <LaunchAgentNode
        label="A"
        config={{ agentType: 'claude', projectName: 'x', projectPath: '/x' }}
        selected={s}
        onClick={vi.fn()}
      />
    )
  ],
  [
    'ScriptNode',
    (s) => <ScriptNode label="S" config={{ scriptType: 'bash' }} selected={s} onClick={vi.fn()} />
  ],
  [
    'ConditionNode',
    (s) => (
      <ConditionNode
        label="C"
        config={{ variable: 'x', operator: 'equals', value: '1' }}
        selected={s}
        onClick={vi.fn()}
      />
    )
  ],
  ['ApprovalNode', (s) => <ApprovalNode label="Ap" config={{}} selected={s} onClick={vi.fn()} />],
  ['LoopNode', (s) => <LoopNode label="L" config={{}} nodes={[]} selected={s} onClick={vi.fn()} />],
  [
    'CreateTaskFromItemNode',
    (s) => <CreateTaskFromItemNode label="Ct" config={{}} selected={s} onClick={vi.fn()} />
  ],
  [
    'CallConnectorActionNode',
    (s) => <CallConnectorActionNode label="Cc" config={{}} selected={s} onClick={vi.fn()} />
  ]
]

describe('node card selection', () => {
  for (const [name, renderCard] of CARDS) {
    it(`${name} marks selection with the shared border, and never the accent`, () => {
      const unselected = render(renderCard(false))
      expect((unselected.container.firstChild as HTMLElement).className).toContain(NODE_UNSELECTED)
      cleanup()

      const selected = render(renderCard(true))
      const root = selected.container.firstChild as HTMLElement
      expect(root.className).toContain(NODE_SELECTED)
      // Bronzo says work is blocked on the person. Selecting a node is neither.
      expect(root.className).not.toContain('bronzo')
      cleanup()
    })
  }
})
