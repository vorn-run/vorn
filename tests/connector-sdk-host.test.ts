import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'
import type { Connector } from '../packages/connector-sdk/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { closeDatabase, initDatabase } from '../packages/server/src/database'
import { stopAllClients } from '../packages/server/src/connectors/mcp-clients'
import { installPack } from '../packages/server/src/connectors/packs'
import { outdatedConnectorMessage } from '../packages/server/src/connectors/sdk-client'
import {
  invokeSdkAction,
  pollSdkConnection,
  preflightSdkConnection,
  sdkConnectionActions
} from '../packages/server/src/connectors/sdk'
import { packConnector } from '../packages/connector-sdk/src/pack'

const SDK_SRC = join(__dirname, '..', 'packages', 'connector-sdk', 'src')
const SDK_FIXTURE = join(__dirname, 'fixtures', 'sdk-connector.ts')
const NATIVE_FIXTURE = join(__dirname, 'fixtures', 'native-connector.mjs')

function connection(name: string, filters: Record<string, unknown>): SourceConnection {
  return {
    id: `conn-${name.toLowerCase().replace(/\W+/g, '-')}`,
    connectorId: 'sdk',
    name,
    filters,
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-13T00:00:00Z'
  } as SourceConnection
}

const command = (args: string[]) => ({ command: process.execPath, args: JSON.stringify(args) })

afterEach(async () => {
  await stopAllClients()
})

describe('an sdk connection run from a command', () => {
  const conn = connection('Fixture', {
    ...command(['--import', 'tsx', SDK_FIXTURE]),
    sdkTrigger: 'tick'
  })

  it('runs an action with typed arguments, and reads back a validation failure', async () => {
    expect(await invokeSdkAction(conn, 'echo', { text: 'hi', count: '3' })).toEqual({
      success: true,
      output: { text: 'hi', count: 3 }
    })
    expect(await invokeSdkAction(conn, 'echo', { count: 3 })).toMatchObject({
      success: false,
      error: expect.stringContaining('requires "text"')
    })
  }, 30_000)

  it('polls its trigger, checks readiness and lists its actions from the child', async () => {
    expect(await pollSdkConnection(conn, 'tick')).toMatchObject({
      events: [{ id: 'tick-1', type: 'mcpPoll', data: { title: 'Tick' } }],
      hasMore: false
    })
    expect(await preflightSdkConnection(conn)).toEqual({ ok: true, message: 'ready' })
    expect((await sdkConnectionActions(conn)).map((action) => action.type)).toEqual([
      'echo',
      'wait'
    ])
  }, 30_000)

  it('says a connector that only speaks MCP was built for an older Vorn', async () => {
    const old = connection('Old one', command([NATIVE_FIXTURE, 'mcp-only']))
    expect(await invokeSdkAction(old, 'echo', {})).toEqual({
      success: false,
      error: outdatedConnectorMessage('Old one')
    })
  }, 30_000)
})

describe('a pack built with the SDK, installed and run', () => {
  let dataDir: string
  let work: string

  beforeAll(() => {
    dataDir = mkdtempSync(join(tmpdir(), 'vorn-sdk-host-data-'))
    work = mkdtempSync(join(tmpdir(), 'vorn-sdk-host-pack-'))
    initDatabase(dataDir)
  })

  afterAll(() => {
    closeDatabase()
    rmSync(dataDir, { recursive: true, force: true })
    rmSync(work, { recursive: true, force: true })
  })

  it('packs, installs, and then lists, polls and acts over the protocol the pack names', async () => {
    const entry = join(work, 'connector.ts')
    writeFileSync(
      entry,
      [
        `import { defineConnector } from ${JSON.stringify(join(SDK_SRC, 'define.ts'))}`,
        'export const connector = defineConnector({',
        "  id: 'host-fixture',",
        "  name: 'Host fixture',",
        "  version: '1.0.0',",
        '  triggers: [',
        '    {',
        "      type: 'tick',",
        "      label: 'Tick',",
        "      poll: () => ({ items: [{ externalId: 'p-1', title: 'Packed', updatedAt: '2026-09-13T00:00:00.000Z' }] })",
        '    }',
        '  ],',
        '  actions: [',
        '    {',
        "      type: 'shout',",
        "      label: 'Shout',",
        "      inputs: [{ key: 'text', label: 'Text', required: true }],",
        '      run: (args) => ({ loud: String(args.text).toUpperCase() })',
        '    }',
        '  ]',
        '})',
        ''
      ].join('\n')
    )
    const { connector } = (await import(entry)) as { connector: Connector }

    const packed = await packConnector(connector, {
      entry,
      resolveDir: work,
      outDir: work,
      sdkModule: join(SDK_SRC, 'server.ts')
    })
    expect(packed.findings.filter((finding) => finding.level === 'error')).toEqual([])
    expect(await installPack({ kind: 'file', path: packed.file as string })).toMatchObject({
      ok: true,
      pack: { id: 'host-fixture', version: '1.0.0', protocol: 1 }
    })

    const conn = connection('Host fixture', {
      sdkConnectorId: 'host-fixture',
      sdkVersion: '1.0.0',
      sdkTrigger: 'tick'
    })
    expect((await sdkConnectionActions(conn)).map((action) => action.type)).toEqual(['shout'])
    expect(await pollSdkConnection(conn, 'tick')).toMatchObject({
      events: [{ id: 'p-1', data: { title: 'Packed' } }],
      hasMore: false
    })
    expect(await invokeSdkAction(conn, 'shout', { text: 'hi' })).toEqual({
      success: true,
      output: { loud: 'HI' }
    })
  }, 60_000)
})
