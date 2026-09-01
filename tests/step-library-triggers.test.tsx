// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [
    { id: 'c1', name: 'owner/repo', connectorId: 'github' },
    { id: 'c2', name: 'Notes MCP', connectorId: 'mcp' }
  ],
  useConnectorIdFor: () => null,
  useConnectionIconFor: () => undefined
}))

const listConnectors = vi.fn(async () => [
  {
    id: 'github',
    name: 'GitHub',
    icon: 'github',
    capabilities: ['triggers'],
    manifest: {
      auth: [],
      triggers: [
        { type: 'issueCreated', label: 'Issue Created', configFields: [] },
        { type: 'prOpened', label: 'PR Opened', configFields: [] }
      ]
    }
  },
  { id: 'mcp', name: 'MCP', icon: 'mcp', capabilities: ['actions'], manifest: { auth: [] } }
])
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  listConnectors,
  listConnectionActions: vi.fn(async () => [])
}

import {
  StepLibrary,
  LibraryPick
} from '../src/renderer/components/workflow-editor/panels/StepLibrary'

afterEach(cleanup)

function renderTriggerLibrary() {
  const onPick = vi.fn()
  const utils = render(
    <StepLibrary
      scope={{ bodyOnly: false, insideBranch: false, triggers: true }}
      onPick={onPick}
      onClose={vi.fn()}
    />
  )
  return { ...utils, onPick }
}

describe('the step library in trigger scope', () => {
  it('lists the built-in trigger types and no steps', async () => {
    renderTriggerLibrary()
    for (const label of [
      'Manual',
      'Recurring schedule',
      'Schedule once',
      'Task created',
      'Task moved',
      'Webhook'
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument()
    }
    expect(screen.queryByText('Agent')).not.toBeInTheDocument()
    expect(screen.queryByText('Parallel branch')).not.toBeInTheDocument()
    expect(screen.getByText('Add a trigger')).toBeInTheDocument()
  })

  it("lists each connection's trigger events under its name", async () => {
    renderTriggerLibrary()
    expect(await screen.findByText('Issue Created')).toBeInTheDocument()
    expect(screen.getByText('PR Opened')).toBeInTheDocument()
    expect(screen.getByText('owner/repo')).toBeInTheDocument()
    // The MCP connection declares no triggers, so it has no group.
    expect(screen.queryByText('Notes MCP')).not.toBeInTheDocument()
  })

  it('returns a preconfigured connector trigger pick', async () => {
    const { onPick } = renderTriggerLibrary()
    fireEvent.click(await screen.findByText('Issue Created'))
    expect(onPick).toHaveBeenCalledWith({
      kind: 'connectorTrigger',
      connectionId: 'c1',
      event: 'issueCreated'
    } satisfies LibraryPick)
  })

  it('returns a built-in trigger type pick', () => {
    const { onPick } = renderTriggerLibrary()
    fireEvent.click(screen.getByText('Webhook'))
    expect(onPick).toHaveBeenCalledWith({
      kind: 'triggerType',
      triggerType: 'webhook'
    } satisfies LibraryPick)
  })

  it('filters triggers by search', async () => {
    renderTriggerLibrary()
    await screen.findByText('Issue Created')
    fireEvent.change(screen.getByPlaceholderText('Search triggers'), {
      target: { value: 'issue' }
    })
    expect(screen.getByText('Issue Created')).toBeInTheDocument()
    expect(screen.queryByText('Webhook')).not.toBeInTheDocument()
  })
})
