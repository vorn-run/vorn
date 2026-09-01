// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { SdkConnectorForm } from '../src/renderer/components/settings/SdkConnectorForm'
import type {
  ConnectorCatalogItem,
  InstalledConnectorPack,
  SdkConnectorManifest
} from '../src/shared/types'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector: (state: unknown) => unknown) =>
    selector({ config: { projects: [{ name: 'vorn', path: '/repo' }] } })
}))

const rpcCall = vi.fn()
vi.mock('../packages/mcp/src/ws-client', () => ({ rpcCall: (...a: unknown[]) => rpcCall(...a) }))

const { registerConnectorTools } = await import('../packages/mcp/src/tools/connectors')

const MANIFEST: SdkConnectorManifest = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  version: '0.5.2',
  triggers: [
    {
      type: 'queryResult',
      label: 'Query result',
      filters: {
        pollTool: 'poll_queryResult',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt',
        titleField: 'title',
        urlField: 'url',
        cursorArg: 'cursor',
        cursorPath: 'nextCursor'
      }
    }
  ],
  actions: [],
  env: []
}

const PACK: InstalledConnectorPack = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  version: '0.5.2',
  path: '/data/connectors/kusto/0.5.2',
  installedAt: 0,
  bytes: 4096,
  triggers: MANIFEST.triggers,
  actions: [],
  env: []
}

const CATALOG_ENTRY: ConnectorCatalogItem = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  description: 'Trigger workflows from the rows a KQL query returns.',
  packageName: '@vornrun/connector-kusto',
  capabilities: ['triggers'],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-kusto'] }
}

describe('SdkConnectorForm probe target', () => {
  const probeSdkConnector = vi.fn()

  beforeEach(() => {
    probeSdkConnector.mockReset().mockResolvedValue({ ok: true, manifest: MANIFEST })
    ;(window as unknown as { api: unknown }).api = {
      probeSdkConnector,
      createConnection: vi.fn(),
      encryptString: vi.fn()
    }
  })

  it('probes the installed files rather than the catalog launch spec', async () => {
    render(
      <SdkConnectorForm
        onDone={vi.fn()}
        onCancel={vi.fn()}
        catalogEntry={CATALOG_ENTRY}
        pack={PACK}
      />
    )
    await waitFor(() =>
      expect(probeSdkConnector).toHaveBeenCalledWith({
        command: 'node',
        args: ['/data/connectors/kusto/0.5.2/index.js']
      })
    )
  })

  it('falls back to the catalog launch spec when nothing is installed', async () => {
    render(<SdkConnectorForm onDone={vi.fn()} onCancel={vi.fn()} catalogEntry={CATALOG_ENTRY} />)
    await waitFor(() => expect(probeSdkConnector).toHaveBeenCalledWith(CATALOG_ENTRY.launch))
  })

  it('probes a side-loaded pack that has no catalog entry at all', async () => {
    render(<SdkConnectorForm onDone={vi.fn()} onCancel={vi.fn()} pack={PACK} />)
    await waitFor(() => expect(probeSdkConnector).toHaveBeenCalledTimes(1))
    expect(probeSdkConnector.mock.calls[0][0].command).toBe('node')
  })
})

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

function collect(): Map<string, Handler> {
  const tools = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      tools.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerConnectorTools(server)
  return tools
}

describe('install_connector with a pack', () => {
  beforeEach(() => {
    rpcCall.mockReset()
  })

  const route = (overrides: Record<string, unknown> = {}) => {
    rpcCall.mockImplementation(async (method: string) => {
      if (method === 'connector:catalog') return { items: [], fetchedAt: 1 }
      if (method === 'connector:installPack')
        return (overrides.install as unknown) ?? { ok: true, pack: PACK }
      if (method === 'connector:probeSdk') return { ok: true, manifest: MANIFEST }
      if (method === 'connection:create') return { id: 'c1' }
      return undefined
    })
  }

  it('installs the pack first, then probes and connects to the installed files', async () => {
    route()
    const result = await collect().get('install_connector')!({
      pack_path: '/tmp/kusto-0.5.2.vorn.tgz'
    })

    const calls = rpcCall.mock.calls.map((call) => call[0])
    expect(calls).toContain('connector:installPack')
    expect(calls.indexOf('connector:installPack')).toBeLessThan(calls.indexOf('connector:probeSdk'))

    const install = rpcCall.mock.calls.find((call) => call[0] === 'connector:installPack')
    expect(install?.[1]).toEqual({ kind: 'file', path: '/tmp/kusto-0.5.2.vorn.tgz' })

    const probe = rpcCall.mock.calls.find((call) => call[0] === 'connector:probeSdk')
    expect(probe?.[1]).toEqual({
      command: 'node',
      args: ['/data/connectors/kusto/0.5.2/index.js']
    })

    const create = rpcCall.mock.calls.find((call) => call[0] === 'connection:create')
    const filters = (create?.[1] as { filters: Record<string, unknown> }).filters
    expect(filters.command).toBe('node')
    expect(filters.args).toBe(JSON.stringify(['/data/connectors/kusto/0.5.2/index.js']))
    expect(filters.sdkConnectorId).toBe('kusto')
    expect(filters.sdkVersion).toBe('0.5.2')

    expect(result.isError).toBeFalsy()
    const payload = JSON.parse(result.content[0].text)
    expect(payload).toMatchObject({ version: '0.5.2', path: '/data/connectors/kusto/0.5.2' })
  })

  it('reports a refused pack and creates no connection', async () => {
    route({ install: { ok: false, error: 'declares dependencies' } })
    const result = await collect().get('install_connector')!({ pack_path: '/tmp/bad.tgz' })

    expect(result.isError).toBe(true)
    expect(result.content[0].text).toContain('declares dependencies')
    expect(rpcCall.mock.calls.map((call) => call[0])).not.toContain('connection:create')
  })

  it('still installs from a package name when no pack path is given', async () => {
    route()
    await collect().get('install_connector')!({ package: '@vornrun/connector-kusto' })

    expect(rpcCall.mock.calls.map((call) => call[0])).not.toContain('connector:installPack')
    const probe = rpcCall.mock.calls.find((call) => call[0] === 'connector:probeSdk')
    expect(probe?.[1]).toEqual({ command: 'npx', args: ['-y', '@vornrun/connector-kusto'] })
  })

  it('names pack_path among the ways to say what to install', async () => {
    route()
    const result = await collect().get('install_connector')!({})
    expect(result.isError).toBe(true)
    expect(result.content[0].text).toContain('pack_path')
  })
})
