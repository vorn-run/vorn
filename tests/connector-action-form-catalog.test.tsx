// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, cleanup, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { CallConnectorActionConfig, ConnectorCatalogItem } from '../src/shared/types'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  iconForConnection: () => undefined
}))

const SLACK: ConnectorCatalogItem = {
  id: 'slack',
  name: 'Slack',
  description: 'Messages and channels',
  packageName: '@vornrun/connector-slack',
  capabilities: ['actions'],
  actions: [
    {
      type: 'post',
      label: 'Post message',
      inputs: [
        { key: 'text', label: 'Text', type: 'string', required: true },
        {
          key: 'channel',
          label: 'Channel',
          type: 'select',
          required: true,
          options: [{ value: 'general', label: 'General' }, { value: 'random' }]
        },
        { key: 'blocks', label: 'Blocks', type: 'json', required: false }
      ]
    }
  ],
  launch: { command: 'npx', args: [] }
}

const listConnectorCatalog = vi.fn(async () => ({
  items: [SLACK],
  templates: [],
  mcpServers: []
}))
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  listConnectionActions: vi.fn(async () => []),
  listConnectorCatalog
}

const { CallConnectorActionNodeForm } =
  await import('../src/renderer/components/workflow-editor/panels/CallConnectorActionNodeForm')
const { __resetCatalogCacheForTests } = await import('../src/renderer/lib/use-connector-catalog')

beforeEach(() => {
  __resetCatalogCacheForTests()
  listConnectorCatalog.mockResolvedValue({ items: [SLACK], templates: [], mcpServers: [] })
})
afterEach(cleanup)

const config = (over: Partial<CallConnectorActionConfig> = {}): CallConnectorActionConfig => ({
  nodeType: 'callConnectorAction',
  connectionId: '',
  connectorId: 'slack',
  action: 'post',
  actionLabel: 'Post message',
  args: {},
  ...over
})

describe('a step configured before its connector arrives', () => {
  it('asks for the arguments the catalog published', async () => {
    render(<CallConnectorActionNodeForm config={config()} onChange={vi.fn()} />)

    expect(await screen.findByText('Text')).toBeInTheDocument()
    expect(screen.getByText('Channel')).toBeInTheDocument()
    expect(screen.getByText('Blocks')).toBeInTheDocument()
  })

  it('says why there is no connection to choose yet', async () => {
    render(<CallConnectorActionNodeForm config={config()} onChange={vi.fn()} />)

    expect(await screen.findByText(/is not installed yet/)).toBeInTheDocument()
  })

  it('asks nothing extra of a step that names no connector', async () => {
    const { container } = render(
      <CallConnectorActionNodeForm config={config({ connectorId: undefined })} onChange={vi.fn()} />
    )
    await Promise.resolve()

    expect(screen.queryByText('Text')).toBeNull()
    expect(container.textContent).toContain('No connections yet')
  })
})
