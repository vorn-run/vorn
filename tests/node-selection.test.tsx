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
import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'

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
    (s) => (
      <ScriptNode
        label="S"
        config={{ scriptType: 'bash', scriptContent: '' }}
        selected={s}
        onClick={vi.fn()}
      />
    )
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
  [
    'LoopNode',
    (s) => (
      <LoopNode
        label="L"
        config={{ nodeType: 'loop', bodyNodeIds: [], maxIterations: 1 }}
        nodes={[]}
        selected={s}
        onClick={vi.fn()}
      />
    )
  ],
  [
    'CreateTaskFromItemNode',
    (s) => (
      <CreateTaskFromItemNode
        label="Ct"
        config={{
          nodeType: 'createTaskFromItem',
          project: 'fromConnection',
          initialStatus: 'todo'
        }}
        selected={s}
        onClick={vi.fn()}
      />
    )
  ],
  [
    'CallConnectorActionNode',
    (s) => (
      <CallConnectorActionNode
        label="Cc"
        config={{ nodeType: 'callConnectorAction', connectionId: 'c1', action: 'run', args: {} }}
        selected={s}
        onClick={vi.fn()}
      />
    )
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

  it('shows its own execution status on every card that has one', () => {
    // The dot is how a run reports itself on the canvas. Three cards had no
    // test rendering them mid-run at all, so a card that silently stopped
    // showing status would have looked like a card that had not started.
    const withStatus: [string, React.ReactElement][] = [
      [
        'LoopNode',
        <LoopNode
          label="L"
          config={{ nodeType: 'loop', bodyNodeIds: [], maxIterations: 1 }}
          nodes={[]}
          executionStatus="running"
          onClick={vi.fn()}
        />
      ],
      [
        'CreateTaskFromItemNode',
        <CreateTaskFromItemNode
          label="Ct"
          config={{
            nodeType: 'createTaskFromItem',
            project: 'fromConnection',
            initialStatus: 'todo'
          }}
          executionStatus="error"
          onClick={vi.fn()}
        />
      ],
      [
        'CallConnectorActionNode',
        <CallConnectorActionNode
          label="Cc"
          config={{ nodeType: 'callConnectorAction', connectionId: 'c1', action: 'run', args: {} }}
          executionStatus="waiting"
          onClick={vi.fn()}
        />
      ]
    ]
    const expected = [
      WORKFLOW_STATUS_DOT.running,
      WORKFLOW_STATUS_DOT.error,
      WORKFLOW_STATUS_DOT.waiting
    ]
    withStatus.forEach(([, element], i) => {
      const { container } = render(element)
      expect(container.querySelector(`.${expected[i]}`)).toBeInTheDocument()
      cleanup()
    })
  })

  it('gives every card the same shell', () => {
    // Two of the eight had drifted to rounded-sm and sat visibly different
    // beside the rest on one canvas. Selection is pinned by the constant above;
    // nothing was watching the shape they are all drawn in.
    const shells = CARDS.map(([, renderCard]) => {
      const { container } = render(renderCard(false))
      const cls = (container.firstChild as HTMLElement).className
      cleanup()
      return ['rounded-md', 'px-3', 'py-2.5', 'w-[280px]'].filter((c) => cls.includes(c))
    })
    for (const found of shells) {
      expect(found).toEqual(['rounded-md', 'px-3', 'py-2.5', 'w-[280px]'])
    }
  })
})
