// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type {
  ConnectorPollTriggerConfig,
  CallConnectorActionConfig,
  CreateTaskFromItemConfig,
  SourceConnection,
  ConnectorManifest
} from '../src/shared/types'

// Zustand store stub: projects for CreateTaskFromItemNodeForm.
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = {
      config: {
        projects: [
          { name: 'vorn', path: '/dev/vorn', preferredAgents: [] },
          { name: 'other', path: '/dev/other', preferredAgents: [] }
        ],
        remoteHosts: []
      }
    }
    return selector ? selector(state) : state
  }
}))

const listConnectorsMock = vi.fn()
const listConnectionsMock = vi.fn()
const listConnectionActionsMock = vi.fn()

beforeEach(() => {
  listConnectorsMock.mockReset()
  listConnectionsMock.mockReset()
  listConnectionActionsMock.mockReset()
  listConnectorsMock.mockResolvedValue([])
  listConnectionsMock.mockResolvedValue([])
  listConnectionActionsMock.mockResolvedValue([])
  Object.defineProperty(window, 'api', {
    configurable: true,
    value: {
      listConnectors: listConnectorsMock,
      listConnections: listConnectionsMock,
      listConnectionActions: listConnectionActionsMock
    }
  })
})

const GITHUB_MANIFEST: ConnectorManifest = {
  auth: [],
  triggers: [
    { type: 'issueCreated', label: 'Issue Created', configFields: [], defaultIntervalMs: 60_000 }
  ],
  actions: [
    {
      type: 'commentOnIssue',
      label: 'Comment on Issue',
      description: 'Post a comment',
      configFields: [
        { key: 'number', label: 'Issue #', type: 'text', required: true },
        { key: 'body', label: 'Comment', type: 'textarea', required: true }
      ]
    }
  ]
}

const CONN: SourceConnection = {
  id: 'conn-1',
  connectorId: 'github',
  name: 'owner/repo',
  filters: {},
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-04-24T00:00:00Z'
}

import { CreateTaskFromItemNodeForm } from '../src/renderer/components/workflow-editor/panels/CreateTaskFromItemNodeForm'
import { ConnectorPollTriggerForm } from '../src/renderer/components/workflow-editor/panels/ConnectorPollTriggerForm'
import { CallConnectorActionNodeForm } from '../src/renderer/components/workflow-editor/panels/CallConnectorActionNodeForm'

describe('CreateTaskFromItemNodeForm', () => {
  const baseConfig: CreateTaskFromItemConfig = {
    nodeType: 'createTaskFromItem',
    project: 'fromConnection',
    initialStatus: 'todo'
  }

  it('renders Project and Initial Status labels', () => {
    const { getByText } = render(
      <CreateTaskFromItemNodeForm config={baseConfig} onChange={() => {}} />
    )
    expect(getByText('Project')).toBeInTheDocument()
    expect(getByText('Initial Status')).toBeInTheDocument()
  })

  it('shows the executionProject helper note for fromConnection', () => {
    const { container } = render(
      <CreateTaskFromItemNodeForm config={baseConfig} onChange={() => {}} />
    )
    expect(container.textContent).toContain('executionProject')
  })

  it('shows the re-sync note under initial status', () => {
    const { container } = render(
      <CreateTaskFromItemNodeForm config={baseConfig} onChange={() => {}} />
    )
    expect(container.textContent).toContain('Local status edits are never overwritten')
  })
})

describe('ConnectorPollTriggerForm', () => {
  const baseConfig: ConnectorPollTriggerConfig = {
    triggerType: 'connectorPoll',
    connectionId: '',
    event: '',
    cron: '*/5 * * * *'
  }

  it('shows the empty-state message when no connections exist', async () => {
    const { getByText } = render(
      <ConnectorPollTriggerForm config={baseConfig} onChange={() => {}} />
    )
    await waitFor(() => {
      expect(getByText(/No connections yet/)).toBeInTheDocument()
    })
  })

  it('renders the cron input and the min/hour/day helper', async () => {
    const { getByPlaceholderText, getByText } = render(
      <ConnectorPollTriggerForm config={baseConfig} onChange={() => {}} />
    )
    expect(getByPlaceholderText('*/5 * * * *')).toBeInTheDocument()
    expect(getByText(/min hour day month weekday/)).toBeInTheDocument()
  })

  it('fires onChange when the cron input is edited', async () => {
    const onChange = vi.fn()
    const { getByPlaceholderText } = render(
      <ConnectorPollTriggerForm config={baseConfig} onChange={onChange} />
    )
    fireEvent.change(getByPlaceholderText('*/5 * * * *'), { target: { value: '0 * * * *' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ cron: '0 * * * *', triggerType: 'connectorPoll' })
    )
  })

  it('renders the Event select only once a connection is selected', async () => {
    listConnectionsMock.mockResolvedValue([CONN])
    listConnectorsMock.mockResolvedValue([
      { id: 'github', name: 'GitHub', icon: 'github', capabilities: [], manifest: GITHUB_MANIFEST }
    ])
    const { queryByText, rerender, getByText } = render(
      <ConnectorPollTriggerForm config={baseConfig} onChange={() => {}} />
    )
    // Before a connection is chosen, the Event section is hidden.
    await waitFor(() => {
      expect(queryByText('Event')).toBeNull()
    })
    rerender(
      <ConnectorPollTriggerForm
        config={{ ...baseConfig, connectionId: 'conn-1' }}
        onChange={() => {}}
      />
    )
    await waitFor(() => {
      expect(getByText('Event')).toBeInTheDocument()
    })
  })
})

describe('CallConnectorActionNodeForm', () => {
  const baseConfig: CallConnectorActionConfig = {
    nodeType: 'callConnectorAction',
    connectionId: '',
    action: '',
    args: {}
  }

  it('shows the empty-state message when no connections exist', async () => {
    const { getByText } = render(
      <CallConnectorActionNodeForm config={baseConfig} onChange={() => {}} />
    )
    await waitFor(() => {
      expect(getByText(/No connections yet/)).toBeInTheDocument()
    })
  })

  it('renders the Connection label', () => {
    const { getByText } = render(
      <CallConnectorActionNodeForm config={baseConfig} onChange={() => {}} />
    )
    expect(getByText('Connection')).toBeInTheDocument()
  })

  it('renders argument inputs for the selected action', async () => {
    listConnectionsMock.mockResolvedValue([CONN])
    listConnectorsMock.mockResolvedValue([
      { id: 'github', name: 'GitHub', icon: 'github', capabilities: [], manifest: GITHUB_MANIFEST }
    ])
    listConnectionActionsMock.mockResolvedValue(GITHUB_MANIFEST.actions ?? [])
    const config: CallConnectorActionConfig = {
      ...baseConfig,
      connectionId: 'conn-1',
      action: 'commentOnIssue'
    }
    const { findByText } = render(
      <CallConnectorActionNodeForm config={config} onChange={() => {}} />
    )
    expect(await findByText('Issue #')).toBeInTheDocument()
    expect(await findByText('Comment')).toBeInTheDocument()
  })

  /** An action whose one argument offers choices. */
  const SELECT_MANIFEST: ConnectorManifest = {
    auth: [],
    triggers: [],
    actions: [
      {
        type: 'post',
        label: 'Post',
        configFields: [
          {
            key: 'level',
            label: 'Level',
            type: 'select',
            options: [
              { value: 'high', label: 'High' },
              { value: 'low', label: 'Low' }
            ]
          }
        ]
      }
    ]
  }

  const renderWithSelect = (level: string) => {
    listConnectionsMock.mockResolvedValue([CONN])
    listConnectionActionsMock.mockResolvedValue(SELECT_MANIFEST.actions ?? [])
    return render(
      <CallConnectorActionNodeForm
        config={{ ...baseConfig, connectionId: 'conn-1', action: 'post', args: { level } }}
        onChange={() => {}}
      />
    )
  }

  it('offers the picker while the value is one of the choices', async () => {
    const { findByText, container } = renderWithSelect('high')
    await findByText('Level')

    expect(container.querySelector('textarea')).toBeNull()
    expect(await findByText('Use a template instead')).toBeInTheDocument()
  })

  it('edits a value the choices do not hold in the template-aware input', async () => {
    // A step is entitled to compute this, so the form must not lose it.
    const { findByText, container } = renderWithSelect('{{steps.pick.level}}')
    await findByText('Level')

    const input = container.querySelector('textarea') as HTMLTextAreaElement
    expect(input).not.toBeNull()
    expect(input.value).toBe('{{steps.pick.level}}')
    expect(await findByText('Choose from the list')).toBeInTheDocument()
  })

  it('swaps to the template input when asked, keeping the value it had', async () => {
    const { findByText, container } = renderWithSelect('high')
    fireEvent.click(await findByText('Use a template instead'))

    const input = container.querySelector('textarea') as HTMLTextAreaElement
    expect(input.value).toBe('high')
  })

  it('calls onChange when an argument value is edited', async () => {
    listConnectionsMock.mockResolvedValue([CONN])
    listConnectorsMock.mockResolvedValue([
      { id: 'github', name: 'GitHub', icon: 'github', capabilities: [], manifest: GITHUB_MANIFEST }
    ])
    listConnectionActionsMock.mockResolvedValue(GITHUB_MANIFEST.actions ?? [])
    const onChange = vi.fn()
    const { findByText, container } = render(
      <CallConnectorActionNodeForm
        config={{ ...baseConfig, connectionId: 'conn-1', action: 'commentOnIssue' }}
        onChange={onChange}
      />
    )
    await findByText('Issue #')
    const issueNumberInput = container.querySelector('textarea') as HTMLTextAreaElement
    fireEvent.change(issueNumberInput, { target: { value: '42' } })
    expect(onChange).toHaveBeenCalled()
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0]
    expect(lastCall.args.number).toBe('42')
  })
})
