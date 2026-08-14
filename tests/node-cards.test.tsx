// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (sel?: (s: unknown) => unknown) => {
    const state = { config: { remoteHosts: [{ id: 'h1', label: 'build-box' }] } }
    return sel ? sel(state) : state
  }
}))
vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnectorIdFor: (id: string | null) => (id ? 'github' : null),
  useConnectionIconFor: () => undefined
}))
vi.mock('../src/renderer/components/ConnectorIcon', () => ({
  ConnectorIcon: () => <span data-testid="connector-icon" />
}))
vi.mock('../src/renderer/components/AgentIcon', () => ({
  AgentIcon: () => <span data-testid="agent-icon" />
}))

import { ScriptNode } from '../src/renderer/components/workflow-editor/nodes/ScriptNode'
import { ConditionNode } from '../src/renderer/components/workflow-editor/nodes/ConditionNode'
import { LaunchAgentNode } from '../src/renderer/components/workflow-editor/nodes/LaunchAgentNode'
import { CallConnectorActionNode } from '../src/renderer/components/workflow-editor/nodes/CallConnectorActionNode'
import { TriggerNode } from '../src/renderer/components/workflow-editor/nodes/TriggerNode'
import type {
  ScriptConfig,
  ConditionConfig,
  LaunchAgentConfig,
  CallConnectorActionConfig,
  TriggerConfig
} from '../src/shared/types'

const noop = (): void => {}

afterEach(cleanup)

describe('ScriptNode', () => {
  it('names the runtime, and the project when it has one', () => {
    render(
      <ScriptNode
        label="Build"
        config={{ scriptType: 'bash', projectName: 'vorn' } as ScriptConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText(/bash/)).toBeInTheDocument()
    expect(screen.getByText(/vorn/)).toBeInTheDocument()
  })

  it('previews the first line that is neither blank nor a comment', () => {
    // A script's opening comment is the least informative line in it.
    render(
      <ScriptNode
        label="Build"
        config={
          { scriptType: 'bash', scriptContent: '# set up\n\n  yarn build\nmore' } as ScriptConfig
        }
        onClick={noop}
      />
    )
    expect(screen.getByText('yarn build')).toBeInTheDocument()
  })

  it('cuts a long preview rather than letting one card run taller', () => {
    render(
      <ScriptNode
        label="Build"
        config={{ scriptType: 'node', scriptContent: 'x'.repeat(80) } as ScriptConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText(`${'x'.repeat(50)}...`)).toBeInTheDocument()
  })
})

describe('ConditionNode', () => {
  it('says so plainly when it has nothing to compare', () => {
    render(<ConditionNode label="If" config={{} as ConditionConfig} onClick={noop} />)
    expect(screen.getByText('Not configured')).toBeInTheDocument()
  })

  it('spells the comparison out, quoting the value it tests against', () => {
    render(
      <ConditionNode
        label="If"
        config={{ variable: 'status', operator: 'equals', value: 'ok' } as ConditionConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('status = "ok"')).toBeInTheDocument()
  })

  it('leaves the value off an operator that does not take one', () => {
    render(
      <ConditionNode
        label="If"
        config={{ variable: 'out', operator: 'isEmpty', value: 'ignored' } as ConditionConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('out is empty')).toBeInTheDocument()
  })
})

describe('LaunchAgentNode', () => {
  it('names the project and branch it will work in', () => {
    render(
      <LaunchAgentNode
        label="Fix it"
        config={{ agentType: 'claude', projectName: 'vorn', branch: 'main' } as LaunchAgentConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText(/vorn/)).toBeInTheDocument()
    expect(screen.getByText(/main/)).toBeInTheDocument()
  })

  it('names the host instead of the branch when the work runs elsewhere', () => {
    // Where it runs is the more surprising fact of the two, and both do not fit.
    render(
      <LaunchAgentNode
        label="Fix it"
        config={
          {
            agentType: 'claude',
            projectName: 'vorn',
            branch: 'main',
            remoteHostId: 'h1'
          } as LaunchAgentConfig
        }
        onClick={noop}
      />
    )
    expect(screen.getByText('build-box')).toBeInTheDocument()
    expect(screen.queryByText(/main/)).not.toBeInTheDocument()
  })

  it('says where its work comes from when there is no prompt to show', () => {
    render(
      <LaunchAgentNode
        label="Next"
        config={{ agentType: 'claude', taskFromQueue: true } as LaunchAgentConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('Next task from queue')).toBeInTheDocument()
    cleanup()

    render(
      <LaunchAgentNode
        label="Task"
        config={{ agentType: 'claude', taskId: 't1' } as LaunchAgentConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('From task')).toBeInTheDocument()
  })

  it('falls back to saying it has no project rather than showing nothing', () => {
    render(
      <LaunchAgentNode
        label="Fix it"
        config={{ agentType: 'claude' } as LaunchAgentConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('No project')).toBeInTheDocument()
  })
})

describe('CallConnectorActionNode', () => {
  it('prompts for an action until one is chosen', () => {
    render(
      <CallConnectorActionNode
        label="Call"
        config={{} as CallConnectorActionConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('Select action')).toBeInTheDocument()
  })

  it('shows the connector it will call once it has a connection', () => {
    render(
      <CallConnectorActionNode
        label="Call"
        config={{ connectionId: 'c1', action: 'createIssue' } as CallConnectorActionConfig}
        onClick={noop}
      />
    )
    expect(screen.getByText('createIssue')).toBeInTheDocument()
    expect(screen.getByTestId('connector-icon')).toBeInTheDocument()
  })
})

describe('TriggerNode', () => {
  it('says what starts the run, in the terms of the trigger', () => {
    const cases: [TriggerConfig, string | RegExp][] = [
      [{ triggerType: 'manual' } as TriggerConfig, 'Click to run'],
      [{ triggerType: 'recurring', cron: '0 9 * * *' } as TriggerConfig, 'Cron: 0 9 * * *'],
      [{ triggerType: 'taskCreated' } as TriggerConfig, 'Any project'],
      [{ triggerType: 'taskCreated', projectFilter: 'vorn' } as TriggerConfig, 'Project: vorn'],
      [{ triggerType: 'taskStatusChanged' } as TriggerConfig, 'Any change'],
      [
        {
          triggerType: 'taskStatusChanged',
          fromStatus: 'todo',
          toStatus: 'done',
          projectFilter: 'vorn'
        } as TriggerConfig,
        /todo → done · vorn/
      ]
    ]

    for (const [config, expected] of cases) {
      render(<TriggerNode label="Start" config={config} onClick={noop} />)
      expect(screen.getByText(expected)).toBeInTheDocument()
      cleanup()
    }
  })

  it("wears the connector's own mark when a connector starts it", () => {
    render(
      <TriggerNode
        label="Start"
        config={
          {
            triggerType: 'connectorPoll',
            connectionId: 'c1',
            event: 'issues.opened',
            cron: '*/5 * * * *'
          } as TriggerConfig
        }
        onClick={noop}
      />
    )
    expect(screen.getByTestId('connector-icon')).toBeInTheDocument()
    expect(screen.getByText(/issues.opened/)).toBeInTheDocument()
  })
})
