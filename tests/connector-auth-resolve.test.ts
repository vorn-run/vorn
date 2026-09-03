import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SdkConnectorAuth } from '../packages/shared/src/types'

// Auth is read in the order the files are: the checkout resolveLaunch would run, else the installed pack.

const pack = { current: undefined as unknown }
const checkout = { launch: undefined as { command: string; args: string[] } | undefined }
const probeSdkConnector = vi.fn()

const CLI: SdkConnectorAuth = {
  rung: 'cli',
  probe: { command: 'glab', args: ['auth', 'status'] },
  borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
}

beforeEach(() => {
  pack.current = undefined
  checkout.launch = undefined
  probeSdkConnector.mockReset()
  vi.doMock('../packages/server/src/logger', () => ({
    default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
  }))
  vi.doMock('../packages/server/src/connectors/packs', () => ({
    installedPack: () => pack.current
  }))
  vi.doMock('../packages/server/src/connectors/catalog', () => ({
    localLaunchSpec: () => checkout.launch
  }))
  vi.doMock('../packages/server/src/connectors/sdk-probe', () => ({ probeSdkConnector }))
  vi.resetModules()
})

afterEach(() => {
  vi.resetModules()
})

const load = async (): Promise<typeof import('../packages/server/src/connectors/connector-auth')> =>
  import('../packages/server/src/connectors/connector-auth')

describe('a connector that is installed', () => {
  it('borrows what its installed manifest declares', async () => {
    pack.current = { auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    const { resolveConnectorAuth } = await load()
    expect(await resolveConnectorAuth('gitlab')).toEqual({
      auth: CLI,
      declared: ['GITLAB_TOKEN'],
      trusted: false
    })
  })

  it('answers for every rung, since the probe and the form read it too', async () => {
    pack.current = { auth: { rung: 'key', keys: ['t'] }, env: [] }
    const { resolveConnectorAuth } = await load()
    expect((await resolveConnectorAuth('gitlab'))?.auth).toEqual({ rung: 'key', keys: ['t'] })
  })
})

describe('a connector being run from a checkout', () => {
  it('asks the checkout what it needs, since nothing is installed to read', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/gitlab/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { id: 'gitlab', auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    })
    const { resolveConnectorAuth } = await load()
    expect(await resolveConnectorAuth('gitlab')).toEqual({
      auth: CLI,
      declared: ['GITLAB_TOKEN'],
      trusted: false
    })
    expect(probeSdkConnector).toHaveBeenCalledWith(checkout.launch)
  })

  it('asks once, then answers from what it was told', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/gitlab/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { id: 'gitlab', auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    })
    const { resolveConnectorAuth } = await load()

    await resolveConnectorAuth('gitlab')
    await resolveConnectorAuth('gitlab')
    expect(probeSdkConnector).toHaveBeenCalledTimes(1)
  })

  it('remembers a checkout that would not start rather than asking before every spawn', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/broken/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({ ok: false, error: 'no manifest tool' })
    const { resolveConnectorAuth } = await load()

    expect(await resolveConnectorAuth('broken')).toBeUndefined()
    expect(await resolveConnectorAuth('broken')).toBeUndefined()
    expect(probeSdkConnector).toHaveBeenCalledTimes(1)
  })
})

describe('a connector that is both installed and checked out', () => {
  it('answers from the checkout, because that is what runs', async () => {
    pack.current = { auth: { rung: 'key', keys: ['t'] }, env: [] }
    checkout.launch = { command: 'node', args: ['/checkout/gitlab/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { id: 'gitlab', auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    })
    const { resolveConnectorAuth } = await load()
    expect(await resolveConnectorAuth('gitlab')).toEqual({
      auth: CLI,
      declared: ['GITLAB_TOKEN'],
      trusted: false
    })
  })
})

describe('a connection that names no packaged connector', () => {
  it('borrows nothing and waits for nothing', async () => {
    const { resolveConnectorAuth } = await load()
    expect(await resolveConnectorAuth('')).toBeUndefined()
  })
})
