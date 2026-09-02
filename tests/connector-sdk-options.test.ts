import { describe, expect, it } from 'vitest'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import {
  createConnectorServer,
  defineConnector,
  OPTIONS_TOOL,
  runOptions
} from '../packages/connector-sdk/src/index'
import type { ActionDefinition, Connector } from '../packages/connector-sdk/src/types'

const withOptions = (extra: Partial<ActionDefinition> = {}): Connector =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    options: {
      channels: () => [
        { value: 'C1', label: 'general' },
        { value: 'C2', label: 'random' }
      ],
      // A bare string is the common case: the value is also what to show.
      levels: () => ['high', 'low']
    },
    actions: [
      {
        type: 'post',
        label: 'Post',
        inputs: [
          { key: 'channel', label: 'Channel', type: 'select', loadOptions: 'channels' },
          {
            key: 'level',
            label: 'Level',
            type: 'select',
            required: true,
            options: [{ value: 'high' }, { value: 'low', label: 'Low priority' }]
          }
        ],
        run: () => ({ ok: true }),
        ...extra
      } as ActionDefinition
    ]
  })

async function connect(connector: Connector): Promise<Client> {
  const server = createConnectorServer(connector, { config: {} })
  const client = new Client({ name: 'test', version: '1.0.0' })
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair()
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)])
  return client
}

describe('choices a connection has to be asked for', () => {
  it('lists a set, taking a bare string as a choice that shows itself', async () => {
    expect(await runOptions(withOptions(), 'channels')).toEqual([
      { value: 'C1', label: 'general' },
      { value: 'C2', label: 'random' }
    ])
    expect(await runOptions(withOptions(), 'levels')).toEqual([{ value: 'high' }, { value: 'low' }])
  })

  it('says so when nothing serves the set that was asked for', async () => {
    await expect(runOptions(withOptions(), 'nope')).rejects.toThrow(/serves no options set "nope"/)
  })

  it('refuses an input pointing at a set the connector does not serve', () => {
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        options: { channels: () => [] },
        actions: [
          {
            type: 'post',
            label: 'Post',
            inputs: [{ key: 'c', label: 'C', type: 'select', loadOptions: 'chanels' }],
            run: () => ({})
          }
        ]
      })
    ).toThrow(/loads options from "chanels", which the connector does not serve/)
  })

  it('serves the set over MCP, and only when the connector has one', async () => {
    const client = await connect(withOptions())
    const names = (await client.listTools()).tools.map((tool) => tool.name)
    expect(names).toContain(OPTIONS_TOOL)

    const result = await client.callTool({ name: OPTIONS_TOOL, arguments: { name: 'channels' } })
    expect((result.structuredContent as { options: unknown[] }).options).toEqual([
      { value: 'C1', label: 'general' },
      { value: 'C2', label: 'random' }
    ])

    const plain = await connect(
      defineConnector({
        id: 'plain',
        name: 'Plain',
        actions: [{ type: 'go', label: 'Go', run: () => ({}) }]
      })
    )
    expect((await plain.listTools()).tools.map((tool) => tool.name)).not.toContain(OPTIONS_TOOL)
  })
})

describe('what the served schema says about an argument', () => {
  it('states fixed choices, so a caller draws a picker rather than a text box', async () => {
    const client = await connect(withOptions())
    const post = (await client.listTools()).tools.find((tool) => tool.name === 'post')
    const properties = (post?.inputSchema as { properties: Record<string, { enum?: string[] }> })
      .properties

    expect(properties.level.enum).toEqual(['high', 'low'])
  })

  it('names the set a dynamic field loads from, since its choices are not known yet', async () => {
    const client = await connect(withOptions())
    const post = (await client.listTools()).tools.find((tool) => tool.name === 'post')
    const properties = (
      post?.inputSchema as { properties: Record<string, { description?: string; enum?: string[] }> }
    ).properties

    expect(properties.channel.enum).toBeUndefined()
    expect(properties.channel.description).toContain('"channels"')
  })
})
