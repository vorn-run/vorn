import { EventEmitter } from 'node:events'
import { PassThrough } from 'node:stream'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AgentModelRequest } from '@vornrun/shared/agent-models'

vi.mock('node:child_process', () => ({ spawn: vi.fn(), execFileSync: vi.fn() }))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: () => ({}) }
}))
import { spawn } from 'node:child_process'
import {
  createModelCatalogService,
  parseModelChoices,
  probeProcess
} from '../packages/server/src/agent-model-catalog'

const request: AgentModelRequest = { agentType: 'codex', projectPath: '/project' }
const config = { command: 'codex', args: [] }
const choices = [{ id: 'm', label: 'Model' }]
afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
})

describe('model catalogs', () => {
  it('normalizes aliases, model IDs, provider IDs, and disabled choices', () => {
    expect(
      parseModelChoices('claude', [
        { value: 'opus[1m]', resolvedModel: 'versioned', displayName: 'Opus' }
      ])
    ).toEqual([{ id: 'opus[1m]', label: 'Opus' }])
    expect(
      parseModelChoices('codex', [
        { id: 'internal', model: 'cli-id', displayName: 'Model' },
        { id: 'hidden', hidden: true }
      ])
    ).toEqual([{ id: 'cli-id', label: 'Model' }])
    expect(
      parseModelChoices('copilot', [
        { modelId: 'auto', name: 'Auto', description: 'Let Copilot pick' },
        {
          modelId: 'opus',
          name: 'Opus',
          _meta: { copilotUsage: '15x', copilotEnablement: 'enabled' }
        },
        { modelId: 'blocked', name: 'Blocked', _meta: { copilotEnablement: 'policy_disabled' } }
      ])
    ).toEqual([
      { id: 'auto', label: 'Auto', description: 'Let Copilot pick' },
      { id: 'opus', label: 'Opus', description: '15x usage' }
    ])
    expect(
      parseModelChoices('opencode', 'provider/model\r\nprovider/model\nother/model\nnoise')
    ).toEqual([
      { id: 'provider/model', label: 'provider/model' },
      { id: 'other/model', label: 'other/model' }
    ])
    expect(() => parseModelChoices('claude', {})).toThrow('Invalid')
  })

  it('deduplicates simultaneous requests, caches, and refreshes explicitly', async () => {
    const discover = vi.fn(async () => choices)
    const service = createModelCatalogService(discover)
    await Promise.all([service(request, config), service(request, config)])
    await service(request, config)
    expect(discover).toHaveBeenCalledTimes(1)
    await service({ ...request, refresh: true }, config)
    expect(discover).toHaveBeenCalledTimes(2)
  })

  it('returns stale choices while refreshing and preserves them on failure', async () => {
    let now = 0
    const discover = vi.fn(async () => choices)
    const service = createModelCatalogService(discover, () => now)
    await service(request, config)
    now = 300_001
    discover.mockRejectedValueOnce(new Error('Sign in to the agent'))
    expect(await service(request, config)).toMatchObject({ choices, status: 'stale' })
    await Promise.resolve()
    expect(discover).toHaveBeenCalledTimes(2)
  })

  it('separates projects and command settings and never probes remotes', async () => {
    const discover = vi.fn(async () => choices)
    const service = createModelCatalogService(discover)
    expect(await service({ ...request, remoteHostId: 'remote' }, config)).toMatchObject({
      choices: [],
      status: 'unavailable'
    })
    expect(await service({ ...request, agentType: 'gemini' }, config)).toMatchObject({
      status: 'unavailable'
    })
    expect(await service({ ...request, projectPath: '{{context.path}}' }, config)).toMatchObject({
      status: 'unavailable'
    })
    expect(discover).not.toHaveBeenCalled()
    await service(request, config)
    await service({ ...request, projectPath: '/another' }, config)
    await service(request, { ...config, args: ['--profile', 'work'] })
    expect(discover).toHaveBeenCalledTimes(3)
  })
})

function childFixture() {
  const child = Object.assign(new EventEmitter(), {
    stdin: new PassThrough(),
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    kill: vi.fn(),
    exitCode: null,
    signalCode: null
  })
  vi.mocked(spawn).mockReturnValue(child as unknown as ReturnType<typeof spawn>)
  const sent: unknown[] = []
  child.stdin.on('data', (chunk) => sent.push(JSON.parse(String(chunk))))
  return {
    child,
    sent,
    reply: (value: unknown) => child.stdout.write(JSON.stringify(value) + '\n')
  }
}
const context = { command: 'agent', args: [], cwd: '/project', env: {} }

describe('discovery process protocol', () => {
  it('initializes Codex, follows pagination, and stops without starting a thread', async () => {
    const { child, sent, reply } = childFixture()
    const result = probeProcess(context, 'codex')
    reply({ id: 1, result: {} })
    reply({ id: 2, result: { data: [{ model: 'first' }], nextCursor: 'next' } })
    reply({ id: 3, result: { data: [{ model: 'second' }], nextCursor: null } })
    expect(await result).toEqual([
      { id: 'first', label: 'first' },
      { id: 'second', label: 'second' }
    ])
    expect(sent).toContainEqual({
      id: 3,
      method: 'model/list',
      params: { limit: 100, includeHidden: false, cursor: 'next' }
    })
    expect(sent.map((value) => (value as { method: string }).method)).toEqual([
      'initialize',
      'initialized',
      'model/list',
      'model/list'
    ])
    expect(child.kill).toHaveBeenCalled()
  })

  it('reads Claude initialization models without sending a user message', async () => {
    const { sent, reply } = childFixture()
    const result = probeProcess(context, 'claude')
    reply({
      type: 'control_response',
      response: { subtype: 'success', response: { models: [{ value: 'sonnet' }] } }
    })
    expect(await result).toEqual([{ id: 'sonnet', label: 'sonnet' }])
    expect(sent).toHaveLength(1)
    expect(spawn).toHaveBeenCalledWith(
      'agent',
      expect.arrayContaining(['--no-session-persistence', '--safe-mode', '--strict-mcp-config']),
      expect.anything()
    )
  })

  it('reads Copilot models from a new session over the agent protocol', async () => {
    const { child, sent, reply } = childFixture()
    const result = probeProcess(context, 'copilot')
    reply({ jsonrpc: '2.0', id: 1, result: { protocolVersion: 1 } })
    reply({
      jsonrpc: '2.0',
      id: 2,
      result: { sessionId: 's', models: { availableModels: [{ modelId: 'auto', name: 'Auto' }] } }
    })
    expect(await result).toEqual([{ id: 'auto', label: 'Auto' }])
    expect(sent.map((value) => (value as { method: string }).method)).toEqual([
      'initialize',
      'session/new'
    ])
    expect(spawn).toHaveBeenCalledWith(
      'agent',
      ['--acp', '--no-remote', '--no-remote-export'],
      expect.anything()
    )
    expect(child.kill).toHaveBeenCalled()
  })

  it('rejects malformed responses and kills timed-out children', async () => {
    vi.useFakeTimers()
    const first = childFixture()
    const malformed = probeProcess(context, 'codex')
    first.child.stdout.write('not JSON\n')
    await expect(malformed).rejects.toThrow('unsupported model response')
    const second = childFixture()
    const pending = probeProcess(context, 'codex')
    await Promise.all([
      expect(pending).rejects.toThrow('timed out'),
      vi.advanceTimersByTimeAsync(15_000)
    ])
    expect(second.child.kill).toHaveBeenCalled()
  })
})
