import { describe, expect, it, vi } from 'vitest'
import type { Client } from '@modelcontextprotocol/sdk/client/index.js'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { SdkCallError } from '../packages/server/src/connectors/native-client'
import { legacyMcpSdkClient, toolArgs } from '../packages/server/src/connectors/sdk-legacy-mcp'

function fake(result: unknown, tools: string[] = []) {
  const client = {
    listTools: vi.fn(async () => ({ tools: tools.map((name) => ({ name })) })),
    callTool: vi.fn(async () => result),
    close: vi.fn(async () => {})
  }
  return { client, adapter: legacyMcpSdkClient(client as unknown as Client) }
}

const text = (value: string) => ({ content: [{ type: 'text', text: value }] })

describe('a connector that only speaks MCP', () => {
  it('says it speaks MCP and offers no hello', () => {
    const { adapter } = fake({})
    expect(adapter.protocol).toBe('mcp')
    expect(adapter.hello).toBeUndefined()
  })

  it('reads its manifest from the structured result, or from the text when that is all it sent', async () => {
    const structured = fake({ structuredContent: { id: 'acme' } })
    expect(await structured.adapter.manifest()).toEqual({ id: 'acme' })
    expect(structured.client.callTool).toHaveBeenCalledWith({
      name: 'vorn_connector_manifest',
      arguments: {}
    })

    expect(await fake(text('{"id":"acme"}')).adapter.manifest()).toEqual({ id: 'acme' })
    await expect(fake(text('not json')).adapter.manifest()).rejects.toThrow(
      'vorn_connector_manifest returned no manifest'
    )
    await expect(fake({ ...text('bad token'), isError: true }).adapter.manifest()).rejects.toThrow(
      'bad token'
    )
  })

  describe('preflight', () => {
    it('reports nothing to check when the connector declares none', async () => {
      const { client, adapter } = fake({})
      expect(await adapter.preflight()).toEqual({ ok: null })
      expect(client.callTool).not.toHaveBeenCalled()
    })

    it('passes on what the connector said', async () => {
      const tools = ['vorn_connector_preflight']
      expect(
        await fake({ structuredContent: { ok: true, message: 'ready' } }, tools).adapter.preflight()
      ).toEqual({ ok: true, message: 'ready' })
      expect(await fake({ structuredContent: { ok: 'yes' } }, tools).adapter.preflight()).toEqual({
        ok: false
      })
    })

    it('fails with the error it printed, or a sentence when it printed none', async () => {
      const tools = ['vorn_connector_preflight']
      expect(
        await fake({ ...text('gh is not signed in'), isError: true }, tools).adapter.preflight()
      ).toEqual({
        ok: false,
        message: 'gh is not signed in'
      })
      expect(await fake({ content: [] }, tools).adapter.preflight()).toEqual({
        ok: false,
        message: 'The connector could not report whether it is ready.'
      })
    })
  })

  describe('actions', () => {
    it('sends every argument as the string the MCP schema expects, leaving out nulls', async () => {
      expect(toolArgs({ s: 'x', n: 3, b: false, o: { a: [1] }, l: ['a'], z: null })).toEqual({
        s: 'x',
        n: '3',
        b: 'false',
        o: '{"a":[1]}',
        l: '["a"]'
      })
      const { client, adapter } = fake({ structuredContent: { done: true } })
      expect(await adapter.action({ action: 'close', args: { id: 7, reason: null } })).toEqual({
        done: true
      })
      expect(client.callTool).toHaveBeenCalledWith({ name: 'close', arguments: { id: '7' } })
    })

    it('returns the whole result when the action declared no output', async () => {
      const raw = text('closed')
      expect(await fake(raw).adapter.action({ action: 'close', args: {} })).toEqual(raw)
    })

    it('fails with the error it printed, carrying what it returned and no kind', async () => {
      const raw = { ...text('not found'), isError: true }
      const failed = fake(raw).adapter.action({ action: 'close', args: {} })
      await expect(failed).rejects.toBeInstanceOf(SdkCallError)
      const error = (await failed.catch((err: unknown) => err)) as SdkCallError
      expect(error).toMatchObject({
        method: 'action/run',
        code: -32000,
        message: 'not found',
        output: raw
      })
      expect(error.kind).toBeUndefined()

      await expect(
        fake({ content: [], isError: true }).adapter.action({ action: 'close', args: {} })
      ).rejects.toThrow('MCP tool close reported an error')
    })

    it('marks a call made through a signed-in window and gives it longer', async () => {
      const { client, adapter } = fake({ structuredContent: {} })
      await adapter.action({ action: 'post', args: { text: 'hi' }, sessionCall: 'key-1' })
      expect(client.callTool).toHaveBeenCalledWith(
        { name: 'post', arguments: { text: 'hi' }, _meta: { 'vorn/sessionCall': 'key-1' } },
        undefined,
        { timeout: 120_000 }
      )
    })
  })

  it('polls a trigger through its tool, with the limit as a string', async () => {
    const items = [{ externalId: '1' }]
    const { client, adapter } = fake({
      structuredContent: { items, nextCursor: 'n2', hasMore: true }
    })
    expect(
      await adapter.poll({ trigger: 'tick', since: 's', cursor: 'n1', limit: 25, sessionCall: 'k' })
    ).toEqual({ items, nextCursor: 'n2', hasMore: true })
    expect(client.callTool).toHaveBeenCalledWith(
      {
        name: 'poll_tick',
        arguments: { since: 's', cursor: 'n1', limit: '25' },
        _meta: { 'vorn/sessionCall': 'k' }
      },
      undefined,
      { timeout: 120_000 }
    )
    expect(await fake({ structuredContent: { items } }).adapter.poll({ trigger: 'tick' })).toEqual({
      items,
      hasMore: false
    })
    await expect(
      fake({ ...text('rate limited'), isError: true }).adapter.poll({ trigger: 'tick' })
    ).rejects.toThrow('rate limited')
  })

  it('lists a field’s choices through the options tool', async () => {
    const options = [{ value: 'c1', label: 'General' }]
    const { client, adapter } = fake({ structuredContent: { options } })
    expect(await adapter.options({ name: 'channels' })).toEqual({ options })
    expect(client.callTool).toHaveBeenCalledWith({
      name: 'vorn_connector_options',
      arguments: { name: 'channels' }
    })
  })

  it('recomputes a footer and runs a link handler under their tool names', async () => {
    const session = { sessionId: 's1', worktreePath: '/w', agent: 'claude' }
    const footer = fake({ structuredContent: { items: [{ label: 'Branch', value: 'main' }] } })
    expect(await footer.adapter.footer({ footer: 'branch', ...session })).toEqual({
      items: [{ label: 'Branch', value: 'main' }]
    })
    expect(footer.client.callTool).toHaveBeenCalledWith({
      name: 'vorn_footer_branch',
      arguments: session
    })

    const handler = fake({ structuredContent: { openPane: 'details' } })
    expect(await handler.adapter.handler({ handler: 'pr', ...session, url: 'https://x' })).toEqual({
      openPane: 'details'
    })
    expect(handler.client.callTool).toHaveBeenCalledWith({
      name: 'vorn_handler_pr',
      arguments: { ...session, url: 'https://x' }
    })
    expect(
      await fake({ structuredContent: { openPane: '' } }).adapter.handler({
        handler: 'pr',
        ...session,
        url: 'https://x'
      })
    ).toEqual({})
  })

  it('closes through its owner when it has one, and the client when it does not', async () => {
    const bare = fake({})
    await bare.adapter.close()
    expect(bare.client.close).toHaveBeenCalled()

    const lifecycle = { close: vi.fn(async () => {}), onExit: vi.fn() }
    const client = { close: vi.fn(async () => {}) }
    const owned = legacyMcpSdkClient(client as unknown as Client, lifecycle)
    const listener = vi.fn()
    owned.onExit(listener)
    await owned.close()
    expect(lifecycle.close).toHaveBeenCalled()
    expect(lifecycle.onExit).toHaveBeenCalledWith(listener)
    expect(client.close).not.toHaveBeenCalled()
  })
})
