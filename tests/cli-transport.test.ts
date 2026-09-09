import { describe, it, expect, vi, beforeEach } from 'vitest'

const rpcCall = vi.hoisted(() => vi.fn(async () => 'called'))
const rpcNotify = vi.hoisted(() => vi.fn(async () => undefined))
const isServerRunning = vi.hoisted(() => vi.fn(() => true))
vi.mock('../packages/server/src/rpc-client', () => ({
  rpcCall,
  rpcNotify,
  isServerRunning,
  useDataDir: vi.fn(),
  dataDir: () => '/tmp/vorn'
}))

import { socketTransport } from '../packages/server/src/cli/transport'
import { clientContext } from '../packages/server/src/cli/deps'
import { parseClientArgs } from '../packages/server/src/client-args'

beforeEach(() => {
  rpcCall.mockClear()
  rpcNotify.mockClear()
  isServerRunning.mockClear()
})

const sink = { write: () => {}, writeErr: () => {} }

describe('the socket transport', () => {
  it('is what a command reaches the server through by default', async () => {
    const ctx = clientContext(sink, parseClientArgs(['session', 'list']))

    await ctx.rpc.call('terminal:listActive')
    await ctx.rpc.notify('terminal:write', { id: 'a', data: 'b' })
    ctx.rpc.isRunning()

    expect(rpcCall).toHaveBeenCalledWith('terminal:listActive', undefined, undefined)
    expect(rpcNotify).toHaveBeenCalledWith('terminal:write', { id: 'a', data: 'b' })
    expect(isServerRunning).toHaveBeenCalled()
  })

  it('carries --timeout into every call without touching the rest', async () => {
    const ctx = clientContext(sink, parseClientArgs(['session', 'list', '--timeout', '250']))

    await ctx.rpc.call('terminal:listActive')
    await ctx.rpc.notify('terminal:write', { id: 'a', data: 'b' })
    expect(ctx.rpc.isRunning()).toBe(true)

    expect(rpcCall).toHaveBeenCalledWith('terminal:listActive', undefined, 250)
    expect(rpcNotify).toHaveBeenCalledWith('terminal:write', { id: 'a', data: 'b' })
  })

  it('passes calls straight through when no ceiling was named', async () => {
    await socketTransport.call('workflow:list')
    expect(rpcCall).toHaveBeenCalledWith('workflow:list', undefined, undefined)
  })
})
