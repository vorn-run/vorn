// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { SdkConnectorForm } from '../src/renderer/components/settings/SdkConnectorForm'
import type { SdkConnectorManifest } from '../src/shared/types'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector: (state: unknown) => unknown) =>
    selector({ config: { projects: [{ name: 'vorn', path: '/repo' }] } })
}))

const probeSdkConnector = vi.fn()
const createConnection = vi.fn()
const encryptString = vi.fn()

const MANIFEST: SdkConnectorManifest = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  version: '0.5.2',
  description: 'Trigger from a KQL query',
  triggers: [
    {
      type: 'queryResult',
      label: 'Query result',
      description: 'Each new row',
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
  actions: [{ type: 'runQuery', label: 'Run query' }],
  env: [
    { name: 'KUSTO_CLUSTER', required: true, secret: false, description: 'Cluster URL' },
    { name: 'KUSTO_APP_KEY', required: false, secret: true }
  ]
}

beforeEach(() => {
  probeSdkConnector.mockReset().mockResolvedValue({ ok: true, manifest: MANIFEST })
  createConnection.mockReset().mockResolvedValue({ id: 'c1' })
  encryptString.mockReset().mockImplementation(async (v: string) => `enc(${v})`)
  ;(window as unknown as { api: unknown }).api = {
    probeSdkConnector,
    createConnection,
    encryptString
  }
})

const setup = () => {
  const onDone = vi.fn()
  const utils = render(<SdkConnectorForm onDone={onDone} onCancel={vi.fn()} />)
  return { ...utils, onDone }
}

/** Type a package name and run the lookup, leaving the manifest on screen. */
async function lookUp(utils: ReturnType<typeof setup>, spec = '@vornrun/connector-kusto') {
  fireEvent.change(utils.getByPlaceholderText('@vornrun/connector-kusto'), {
    target: { value: spec }
  })
  fireEvent.click(utils.getByText('Look up'))
  await waitFor(() => expect(utils.getByText('Azure Data Explorer')).toBeInTheDocument())
}

describe('SdkConnectorForm', () => {
  it('runs a package name through npx when looking it up', async () => {
    const utils = setup()

    await lookUp(utils)

    expect(probeSdkConnector).toHaveBeenCalledWith({
      command: 'npx',
      args: ['-y', '@vornrun/connector-kusto']
    })
  })

  it('shows the connector its own way, not as a generic MCP server', async () => {
    const utils = setup()

    await lookUp(utils)

    expect(utils.getByText('v0.5.2')).toBeInTheDocument()
    expect(utils.getByText('Trigger from a KQL query')).toBeInTheDocument()
    expect(utils.getByText('Each new row')).toBeInTheDocument()
    expect(utils.getByText('1 action available to workflow steps.')).toBeInTheDocument()
  })

  it('asks only for the environment variables the connector declared', async () => {
    const utils = setup()

    await lookUp(utils)

    expect(utils.getByText('KUSTO_CLUSTER')).toBeInTheDocument()
    expect(utils.getByText('Cluster URL')).toBeInTheDocument()
    expect(utils.getByText('KUSTO_APP_KEY')).toBeInTheDocument()
    // The eight polling fields come from the manifest, so they are never asked for.
    expect(utils.queryByText('itemsPath')).not.toBeInTheDocument()
    expect(utils.queryByText('pollTool')).not.toBeInTheDocument()
  })

  it('reports a probe failure instead of offering a connection that cannot work', async () => {
    probeSdkConnector.mockResolvedValue({ ok: false, error: 'does not describe itself' })
    const utils = setup()

    fireEvent.change(utils.getByPlaceholderText('@vornrun/connector-kusto'), {
      target: { value: 'some-mcp-server' }
    })
    fireEvent.click(utils.getByText('Look up'))

    await waitFor(() => expect(utils.getByText('does not describe itself')).toBeInTheDocument())
    expect(utils.queryByText('Azure Data Explorer')).not.toBeInTheDocument()
  })

  it('keeps Connect disabled until every required variable is filled', async () => {
    const utils = setup()
    await lookUp(utils)

    expect(utils.getByText('Connect')).toBeDisabled()

    const inputs = utils.container.querySelectorAll('input')
    // Filling the optional secret alone is not enough.
    fireEvent.change(inputs[2], { target: { value: 'secret' } })
    expect(utils.getByText('Connect')).toBeDisabled()

    fireEvent.change(inputs[1], { target: { value: 'https://help.kusto.windows.net' } })
    expect(utils.getByText('Connect')).toBeEnabled()

    // Whitespace does not count as a value.
    fireEvent.change(inputs[1], { target: { value: '   ' } })
    expect(utils.getByText('Connect')).toBeDisabled()
  })

  it('fills the polling fields from the manifest so nobody transcribes them', async () => {
    const utils = setup()
    await lookUp(utils)

    const inputs = utils.container.querySelectorAll('input')
    // [0] is the package spec; the declared env vars follow in order.
    fireEvent.change(inputs[1], { target: { value: 'https://help.kusto.windows.net' } })
    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    const arg = createConnection.mock.calls[0][0]
    expect(arg.connectorId).toBe('mcp')
    expect(arg.name).toBe('Azure Data Explorer: Query result')
    expect(arg.filters).toMatchObject({
      command: 'npx',
      args: JSON.stringify(['-y', '@vornrun/connector-kusto']),
      env: JSON.stringify({ KUSTO_CLUSTER: 'https://help.kusto.windows.net' }),
      sdkConnectorId: 'kusto',
      sdkVersion: '0.5.2',
      pollTool: 'poll_queryResult',
      itemsPath: 'items',
      idField: 'externalId',
      timestampField: 'updatedAt',
      cursorArg: 'cursor',
      cursorPath: 'nextCursor'
    })
  })

  it('encrypts a secret variable before it reaches the database', async () => {
    const utils = setup()
    await lookUp(utils)

    const inputs = utils.container.querySelectorAll('input')
    fireEvent.change(inputs[1], { target: { value: 'https://help.kusto.windows.net' } })
    fireEvent.change(inputs[2], { target: { value: 'super-secret' } })
    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    expect(encryptString).toHaveBeenCalledWith(JSON.stringify({ KUSTO_APP_KEY: 'super-secret' }))
    const { filters } = createConnection.mock.calls[0][0]
    expect(filters.secretEnv).toBe(`enc(${JSON.stringify({ KUSTO_APP_KEY: 'super-secret' })})`)
    // The secret must not also travel in the plaintext env blob.
    expect(filters.env).toBe(JSON.stringify({ KUSTO_CLUSTER: 'https://help.kusto.windows.net' }))
  })

  it('omits secretEnv entirely when the connector needs no secrets', async () => {
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { ...MANIFEST, env: [MANIFEST.env[0]] }
    })
    const utils = setup()
    await lookUp(utils)

    const inputs = utils.container.querySelectorAll('input')
    fireEvent.change(inputs[1], { target: { value: 'https://help.kusto.windows.net' } })
    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    expect(createConnection.mock.calls[0][0].filters.secretEnv).toBeUndefined()
    expect(encryptString).not.toHaveBeenCalled()
  })

  it('applies the filters of whichever trigger the user picks', async () => {
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: {
        ...MANIFEST,
        env: [],
        triggers: [
          MANIFEST.triggers[0],
          {
            type: 'alerts',
            label: 'Alerts',
            filters: { ...MANIFEST.triggers[0].filters, pollTool: 'poll_alerts' }
          }
        ]
      }
    })
    const utils = setup()
    await lookUp(utils)

    fireEvent.change(utils.getByDisplayValue('Query result'), { target: { value: 'alerts' } })
    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    const arg = createConnection.mock.calls[0][0]
    expect(arg.filters.pollTool).toBe('poll_alerts')
    expect(arg.name).toBe('Azure Data Explorer: Alerts')
  })

  it('stores the connector glyph on the connection so it is recognizable later', async () => {
    const icon = { viewBox: '0 0 16 16', paths: ['M1 1h4v4z'] }
    probeSdkConnector.mockResolvedValue({ ok: true, manifest: { ...MANIFEST, env: [], icon } })
    const utils = setup()
    await lookUp(utils)

    // The glyph stands in for the generic check mark once one is available.
    const drawn = [...utils.container.querySelectorAll('path')].map((p) => p.getAttribute('d'))
    expect(drawn).toContain('M1 1h4v4z')

    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    expect(createConnection.mock.calls[0][0].filters.sdkIcon).toBe(JSON.stringify(icon))
  })

  it('omits sdkIcon entirely for a connector that ships no glyph', async () => {
    probeSdkConnector.mockResolvedValue({ ok: true, manifest: { ...MANIFEST, env: [] } })
    const utils = setup()
    await lookUp(utils)

    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    expect(createConnection.mock.calls[0][0].filters).not.toHaveProperty('sdkIcon')
  })

  it('surfaces a failed connection attempt rather than closing the form', async () => {
    createConnection.mockRejectedValue(new Error('database is locked'))
    probeSdkConnector.mockResolvedValue({ ok: true, manifest: { ...MANIFEST, env: [] } })
    const utils = setup()
    await lookUp(utils)

    fireEvent.click(utils.getByText('Connect'))

    await waitFor(() => expect(utils.getByText('database is locked')).toBeInTheDocument())
    expect(utils.onDone).not.toHaveBeenCalled()
  })
})
