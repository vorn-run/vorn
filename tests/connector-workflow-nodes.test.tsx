// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Simple zustand stub — the node components only read config for icon/label.
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = { config: { remoteHosts: [] } }
    return selector ? selector(state) : state
  }
}))

// Nodes call window.api.listConnections to resolve the connector brand icon.
const listConnectionsMock = vi.fn().mockResolvedValue([])
beforeEach(() => {
  listConnectionsMock.mockResolvedValue([])
  Object.defineProperty(window, 'api', {
    configurable: true,
    value: { listConnections: listConnectionsMock }
  })
})

import { CreateTaskFromItemNode } from '../src/renderer/components/workflow-editor/nodes/CreateTaskFromItemNode'
import { CallConnectorActionNode } from '../src/renderer/components/workflow-editor/nodes/CallConnectorActionNode'
import { TriggerNode } from '../src/renderer/components/workflow-editor/nodes/TriggerNode'
import { NODE_SELECTED } from '../src/renderer/components/workflow-editor/node-visuals'
import type {
  CreateTaskFromItemConfig,
  CallConnectorActionConfig,
  ConnectorPollTriggerConfig,
  TriggerConfig
} from '../src/shared/types'

describe('CreateTaskFromItemNode', () => {
  const config: CreateTaskFromItemConfig = {
    nodeType: 'createTaskFromItem',
    project: 'fromConnection',
    initialStatus: 'todo'
  }

  it('renders the label and an svg icon', () => {
    const { getByText, container } = render(
      <CreateTaskFromItemNode label="Create task from item" config={config} onClick={() => {}} />
    )
    expect(getByText('Create task from item')).toBeInTheDocument()
    expect(container.querySelector('svg')).toBeInTheDocument()
  })

  it('shows Project from connection + initial status in the subtitle', () => {
    const { getByText } = render(
      <CreateTaskFromItemNode label="n" config={config} onClick={() => {}} />
    )
    expect(getByText(/Project from connection/)).toBeInTheDocument()
    expect(getByText(/initial: todo/)).toBeInTheDocument()
  })

  it('uses the project name verbatim when not fromConnection', () => {
    const { getByText } = render(
      <CreateTaskFromItemNode
        label="n"
        config={{ ...config, project: 'myproj' }}
        onClick={() => {}}
      />
    )
    expect(getByText(/myproj/)).toBeInTheDocument()
  })

  it('fires onClick when the card is clicked', () => {
    const onClick = vi.fn()
    const { container } = render(
      <CreateTaskFromItemNode label="n" config={config} onClick={onClick} />
    )
    fireEvent.click(container.firstChild as Element)
    expect(onClick).toHaveBeenCalled()
  })

  it('applies the selected style when selected', () => {
    const { container } = render(
      <CreateTaskFromItemNode label="n" config={config} selected onClick={() => {}} />
    )
    expect((container.firstChild as HTMLElement).className).toContain(NODE_SELECTED)
  })
})

describe('CallConnectorActionNode', () => {
  const config: CallConnectorActionConfig = {
    nodeType: 'callConnectorAction',
    connectionId: 'conn-1',
    action: 'commentOnIssue',
    args: {}
  }

  it('renders the action name as subtitle', () => {
    const { getByText } = render(
      <CallConnectorActionNode label="Post comment" config={config} onClick={() => {}} />
    )
    expect(getByText('Post comment')).toBeInTheDocument()
    expect(getByText('commentOnIssue')).toBeInTheDocument()
  })

  it('shows "Select action" when no action is chosen yet', () => {
    const { getByText } = render(
      <CallConnectorActionNode label="n" config={{ ...config, action: '' }} onClick={() => {}} />
    )
    expect(getByText('Select action')).toBeInTheDocument()
  })

  it('invokes onClick on click', () => {
    const onClick = vi.fn()
    const { container } = render(
      <CallConnectorActionNode label="n" config={config} onClick={onClick} />
    )
    fireEvent.click(container.firstChild as Element)
    expect(onClick).toHaveBeenCalled()
  })
})

describe('TriggerNode', () => {
  it('renders a manual trigger subtitle', () => {
    const cfg: TriggerConfig = { triggerType: 'manual' }
    const { getByText } = render(<TriggerNode label="Kick off" config={cfg} onClick={() => {}} />)
    expect(getByText('Click to run')).toBeInTheDocument()
  })

  it('renders a recurring trigger with its cron expression', () => {
    const cfg: TriggerConfig = { triggerType: 'recurring', cron: '0 9 * * 1-5' }
    const { getByText } = render(<TriggerNode label="Weekdays" config={cfg} onClick={() => {}} />)
    expect(getByText(/0 9 \* \* 1-5/)).toBeInTheDocument()
  })

  it('renders a connectorPoll trigger with event + cron in the subtitle', () => {
    const cfg: ConnectorPollTriggerConfig = {
      triggerType: 'connectorPoll',
      connectionId: 'conn-1',
      event: 'issueCreated',
      cron: '*/5 * * * *'
    }
    const { getByText } = render(<TriggerNode label="Poll" config={cfg} onClick={() => {}} />)
    expect(getByText(/issueCreated/)).toBeInTheDocument()
    expect(getByText(/\*\/5/)).toBeInTheDocument()
  })

  it('renders a taskStatusChanged trigger with the from → to transition', () => {
    const cfg: TriggerConfig = {
      triggerType: 'taskStatusChanged',
      fromStatus: 'todo',
      toStatus: 'in_progress'
    }
    const { getByText } = render(
      <TriggerNode label="On in progress" config={cfg} onClick={() => {}} />
    )
    expect(getByText(/todo → in_progress/)).toBeInTheDocument()
  })

  it('renders a taskCreated trigger with project filter', () => {
    const cfg: TriggerConfig = { triggerType: 'taskCreated', projectFilter: 'my-proj' }
    const { getByText } = render(<TriggerNode label="Created" config={cfg} onClick={() => {}} />)
    expect(getByText(/my-proj/)).toBeInTheDocument()
  })

  it('fires onClick and stops propagation', () => {
    const onClick = vi.fn()
    const parentClick = vi.fn()
    const { container } = render(
      <div data-testid="parent" onClick={parentClick}>
        <TriggerNode label="n" config={{ triggerType: 'manual' }} onClick={onClick} />
      </div>
    )
    // The TriggerNode's clickable root is the first child of the wrapper.
    const parent = container.querySelector('[data-testid="parent"]')!
    fireEvent.click(parent.firstChild as Element)
    expect(onClick).toHaveBeenCalled()
    expect(parentClick).not.toHaveBeenCalled()
  })
})
