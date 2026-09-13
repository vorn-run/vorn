import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/process-utils', () => ({
  getSafeEnv: () => ({
    PATH: process.env.PATH ?? '',
    HOME: process.env.HOME ?? '',
    TMPDIR: process.env.TMPDIR ?? ''
  })
}))

import log from '../packages/server/src/logger'
import {
  SDK_TIMEOUTS_MS,
  SdkDetectionError,
  connectSdkClient,
  detectionFor,
  helloTimeoutFor,
  type SdkClient,
  type SdkLaunch
} from '../packages/server/src/connectors/sdk-client'
import type { LaunchSource } from '../packages/server/src/connectors/mcp-clients'

const REPO = path.join(__dirname, '..')
const NATIVE = path.join(__dirname, 'fixtures', 'native-connector.mjs')
const SDK = path.join(__dirname, 'fixtures', 'sdk-connector.ts')

const opened: SdkClient[] = []

function native(mode: string, source: LaunchSource = 'command', protocol?: number): SdkLaunch {
  return {
    command: process.execPath,
    args: [NATIVE, mode],
    source,
    env: {},
    ...(protocol !== undefined && { protocol })
  }
}

function sdk(): SdkLaunch {
  return {
    command: process.execPath,
    args: ['--import', 'tsx', SDK],
    source: 'checkout',
    env: {},
    cwd: REPO
  }
}

async function connect(
  launch: SdkLaunch,
  options: Partial<Parameters<typeof connectSdkClient>[1]> = {}
): Promise<SdkClient> {
  const client = await connectSdkClient(launch, { label: 'connectors', key: 'fixture', ...options })
  opened.push(client)
  return client
}

function fakeLegacy() {
  const client = { protocol: 'mcp', close: vi.fn(async () => {}) } as unknown as SdkClient
  return { client, start: vi.fn<(launch: SdkLaunch) => Promise<SdkClient>>(async () => client) }
}

beforeEach(() => {
  vi.mocked(log.info).mockClear()
})

afterEach(async () => {
  await Promise.all(opened.splice(0).map((client) => client.close()))
})

describe('deciding how to talk to a connector', () => {
  it.each([
    [{ source: 'pack' as const }, 'mcp'],
    [{ source: 'pack' as const, protocol: 1 }, 'native'],
    [{ source: 'pack' as const, protocol: 2 }, 'unsupported'],
    [{ source: 'checkout' as const }, 'probe'],
    [{ source: 'command' as const }, 'probe'],
    [{ source: 'command' as const, protocol: 2 }, 'probe']
  ])('%o is %s', (launch, expected) => {
    expect(detectionFor(launch)).toBe(expected)
  })

  it('waits longer for npx, which may download the package first', () => {
    expect(helloTimeoutFor('npx')).toBe(90_000)
    expect(helloTimeoutFor('/usr/local/bin/npx')).toBe(90_000)
    expect(helloTimeoutFor('C:\\Program Files\\nodejs\\NPX.CMD')).toBe(90_000)
    expect(helloTimeoutFor('npx.exe')).toBe(90_000)
    expect(helloTimeoutFor('node')).toBe(15_000)
    expect(helloTimeoutFor('/opt/npx-tools/run')).toBe(15_000)
    expect(SDK_TIMEOUTS_MS).toEqual({
      hello: 15_000,
      npxHello: 90_000,
      manifest: 15_000,
      call: 60_000,
      sessionCall: 120_000
    })
  })
})

describe('an installed pack', () => {
  it('that names protocol 1 is spoken to natively', async () => {
    const legacy = fakeLegacy()
    const client = await connect(native('normal', 'pack', 1), { startLegacy: legacy.start })
    expect(client.protocol).toBe(1)
    expect(client.hello?.connector.id).toBe('native-fixture')
    expect(client.exited).toBe(false)
    expect(await client.manifest()).toMatchObject({ id: 'native-fixture', protocol: 1 })
    expect(legacy.start).not.toHaveBeenCalled()
    await client.close()
    expect(client.exited).toBe(true)
  })

  it('that names no protocol is started as MCP without a hello', async () => {
    const legacy = fakeLegacy()
    const launch: SdkLaunch = { command: '/no/such/connector', args: [], source: 'pack', env: {} }
    expect(await connect(launch, { startLegacy: legacy.start })).toBe(legacy.client)
    expect(legacy.start).toHaveBeenCalledWith(launch)
    expect(log.info).toHaveBeenCalledWith('[connectors] fixture: legacy MCP protocol (pack)')
  })

  it('that names a newer protocol is refused before it starts', async () => {
    const legacy = fakeLegacy()
    const launch: SdkLaunch = {
      command: '/no/such/connector',
      args: [],
      source: 'pack',
      env: {},
      protocol: 2
    }
    const refused = connectSdkClient(launch, {
      label: 'connectors',
      key: 'fixture',
      startLegacy: legacy.start
    })
    await expect(refused).rejects.toBeInstanceOf(SdkDetectionError)
    await expect(refused).rejects.toMatchObject({
      reason: 'unsupported',
      message: 'fixture speaks connector protocol 2, which needs a newer Vorn'
    })
    expect(legacy.start).not.toHaveBeenCalled()
  })

  it('that names protocol 1 but only speaks MCP fails rather than falling back', async () => {
    const legacy = fakeLegacy()
    const failed = connectSdkClient(native('mcp-only', 'pack', 1), {
      label: 'connectors',
      key: 'fixture',
      startLegacy: legacy.start
    })
    await expect(failed).rejects.toMatchObject({
      reason: 'failed',
      message: 'fixture did not answer vorn/hello: Method not found'
    })
    expect(legacy.start).not.toHaveBeenCalled()
  })
})

describe('a checkout or a stored command', () => {
  it('is spoken to natively when it answers the hello', async () => {
    const client = await connect(native('normal'))
    expect(client.protocol).toBe(1)
    expect(await client.preflight()).toEqual({ ok: true, message: 'ready' })
    expect(await client.options({ name: 'any' })).toEqual({ options: [{ value: 'a', label: 'A' }] })
    expect(await client.poll({ trigger: 'tick', cursor: 'c1' })).toMatchObject({
      items: [{ externalId: '1', cursor: 'c1' }],
      hasMore: false
    })
    expect(
      await client.action({ action: 'echo', args: { n: 3, list: [1, 'a'] }, sessionCall: 'k' })
    ).toEqual({ echo: { n: 3, list: [1, 'a'] }, sessionCall: 'k' })
    expect(
      await client.footer({ footer: 'branch', sessionId: 's', worktreePath: '/w', agent: 'claude' })
    ).toEqual({ items: [{ label: 'Branch', value: 'main' }] })
    expect(
      await client.handler({
        handler: 'pr',
        sessionId: 's',
        worktreePath: '/w',
        agent: 'claude',
        url: 'https://example.com'
      })
    ).toEqual({ openPane: 'details' })
    expect(log.info).not.toHaveBeenCalledWith(expect.stringContaining('legacy MCP protocol'))
  })

  it('refuses a native answer that is not the shape the method promises', async () => {
    const client = await connect(native('normal'))
    await expect(client.poll({ trigger: 'malformed' })).rejects.toMatchObject({
      method: 'trigger/poll',
      message: 'fixture answered trigger/poll without a page of items'
    })
  })

  it('gives a call made through a signed-in window longer to answer', async () => {
    const client = await connect(native('slow'), { timeouts: { call: 50, sessionCall: 5_000 } })
    await expect(client.action({ action: 'echo', args: {} })).rejects.toMatchObject({
      reason: 'timeout'
    })
    expect(await client.action({ action: 'echo', args: {}, sessionCall: 'k' })).toMatchObject({
      sessionCall: 'k'
    })
  })

  it('that turns out to be an MCP connector is spoken to through the MCP adapter', async () => {
    const legacy = fakeLegacy()
    expect(await connect(native('mcp-only'), { startLegacy: legacy.start })).toBe(legacy.client)
    expect(log.info).toHaveBeenCalledWith('[connectors] fixture: legacy MCP protocol (command)')
  })

  it('that knows the hello but not this protocol needs a newer Vorn', async () => {
    const legacy = fakeLegacy()
    await expect(
      connectSdkClient(native('hello-unsupported'), {
        label: 'connectors',
        key: 'fixture',
        startLegacy: legacy.start
      })
    ).rejects.toMatchObject({
      reason: 'unsupported',
      message: 'fixture speaks a newer connector protocol, which needs a newer Vorn'
    })
    expect(legacy.start).not.toHaveBeenCalled()
  })

  it('fails loudly when the hello errors, times out or the child dies', async () => {
    const legacy = fakeLegacy()
    const options = { label: 'connectors', key: 'fixture', startLegacy: legacy.start }
    await expect(connectSdkClient(native('hello-error'), options)).rejects.toMatchObject({
      reason: 'failed',
      message: 'fixture did not answer vorn/hello: the fixture refused to start'
    })
    await expect(
      connectSdkClient(native('silent'), { ...options, timeouts: { hello: 200 } })
    ).rejects.toMatchObject({
      reason: 'failed',
      message: expect.stringContaining('did not answer vorn/hello within')
    })
    await expect(connectSdkClient(native('crash-on-start'), options)).rejects.toMatchObject({
      reason: 'failed',
      message: expect.stringContaining('Error: the fixture could not start')
    })
    expect(legacy.start).not.toHaveBeenCalled()
  })
})

describe('a connector built with the SDK', () => {
  it('is spoken to natively from a checkout, with typed values both ways', async () => {
    const client = await connect(sdk(), { key: 'sdk-fixture' })
    expect(client.protocol).toBe(1)
    expect(client.hello).toMatchObject({
      sdk: { name: '@vornrun/connector-sdk' },
      connector: { id: 'sdk-fixture', version: '1.0.0', kind: 'connector' }
    })
    expect(await client.manifest()).toMatchObject({
      protocol: 1,
      id: 'sdk-fixture',
      name: 'SDK fixture'
    })
    expect(await client.preflight()).toEqual({ ok: true, message: 'ready' })
    expect(await client.poll({ trigger: 'tick', limit: 5 })).toMatchObject({
      items: [{ externalId: 'tick-1', limit: 5 }],
      hasMore: false
    })
    expect(await client.action({ action: 'echo', args: { text: 'hi', count: '3' } })).toEqual({
      text: 'hi',
      count: 3
    })
    await expect(client.action({ action: 'echo', args: { count: 3 } })).rejects.toMatchObject({
      method: 'action/run',
      kind: 'validation',
      field: 'text'
    })
    expect(log.info).not.toHaveBeenCalledWith(expect.stringContaining('legacy MCP protocol'))
    await client.close()
    expect(client.exited).toBe(true)
  }, 30_000)
})
