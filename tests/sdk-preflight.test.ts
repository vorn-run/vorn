import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const { preflight, getOrStartSdkClient } = vi.hoisted(() => {
  const preflight = vi.fn()
  return { preflight, getOrStartSdkClient: vi.fn(async () => ({ preflight })) }
})
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({ getOrStartSdkClient }))

const { preflightSdkConnection } = await import('../packages/server/src/connectors/sdk')
const { SdkDetectionError, outdatedConnectorMessage } =
  await import('../packages/server/src/connectors/sdk-client')

const conn = { id: 'c1', connectorId: 'sdk', filters: {} } as unknown as SourceConnection

beforeEach(() => {
  preflight.mockReset()
  getOrStartSdkClient.mockReset().mockImplementation(async () => ({ preflight }))
})

describe('preflightSdkConnection', () => {
  // "Nothing to check" and "checked, fine" are different answers; only one reads as reassurance.
  it('reports null when the connector declares no preflight', async () => {
    preflight.mockResolvedValue({ ok: null })
    expect(await preflightSdkConnection(conn)).toEqual({ ok: null })
  })

  it('reports a passing check', async () => {
    preflight.mockResolvedValue({ ok: true })
    expect(await preflightSdkConnection(conn)).toEqual({ ok: true })
  })

  it('carries the message, which is the part a user can act on', async () => {
    preflight.mockResolvedValue({ ok: false, message: 'Run `gh auth login`.' })
    expect(await preflightSdkConnection(conn)).toEqual({
      ok: false,
      message: 'Run `gh auth login`.'
    })
  })

  // The RPC turns a throw into a failed check, never into "nothing to check".
  it('rejects when the connector will not launch', async () => {
    getOrStartSdkClient.mockRejectedValue(new Error('spawn npx ENOENT'))
    await expect(preflightSdkConnection(conn)).rejects.toThrow('spawn npx ENOENT')
  })

  it('says how to fix a pack built for an older Vorn', async () => {
    getOrStartSdkClient.mockRejectedValue(
      new SdkDetectionError('outdated', outdatedConnectorMessage('Substack'))
    )
    await expect(preflightSdkConnection(conn)).rejects.toThrow(
      'Substack was built for an older Vorn. Update it in Settings → Connectors'
    )
  })
})
