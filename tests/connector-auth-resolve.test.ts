import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SdkConnectorAuth } from '../packages/shared/src/types'

/**
 * Where a connector's auth is read from, in the order its files are.
 *
 * `resolveLaunch` prefers a checkout over an installed pack, so the auth has to
 * as well: a connector being developed has no installed manifest, and reading
 * only that one would leave it borrowing nothing and reporting that its rung
 * has nothing to check.
 */

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
    describePack: () => pack.current
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
    const { mightBorrow, resolveBorrow } = await load()
    expect(mightBorrow('gitlab')).toBe(true)
    expect(await resolveBorrow('gitlab')).toEqual({ auth: CLI, declared: ['GITLAB_TOKEN'] })
  })

  it('is not worth suspending for when it borrows nothing', async () => {
    pack.current = { auth: { rung: 'key', keys: ['t'] }, env: [] }
    const { mightBorrow, resolveBorrow } = await load()
    expect(mightBorrow('gitlab')).toBe(false)
    expect(await resolveBorrow('gitlab')).toBeUndefined()
  })
})

describe('a connector being run from a checkout', () => {
  it('asks the checkout what it needs, since nothing is installed to read', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/gitlab/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { id: 'gitlab', auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    })
    const { mightBorrow, resolveBorrow } = await load()

    expect(mightBorrow('gitlab')).toBe(true)
    expect(await resolveBorrow('gitlab')).toEqual({ auth: CLI, declared: ['GITLAB_TOKEN'] })
    expect(probeSdkConnector).toHaveBeenCalledWith(checkout.launch)
  })

  it('asks once, then answers from what it was told', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/gitlab/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: { id: 'gitlab', auth: CLI, env: [{ name: 'GITLAB_TOKEN' }] }
    })
    const { resolveBorrow } = await load()

    await resolveBorrow('gitlab')
    await resolveBorrow('gitlab')
    expect(probeSdkConnector).toHaveBeenCalledTimes(1)
  })

  it('remembers a checkout that would not start rather than asking before every spawn', async () => {
    checkout.launch = { command: 'node', args: ['/checkout/broken/dist/index.js'] }
    probeSdkConnector.mockResolvedValue({ ok: false, error: 'no manifest tool' })
    const { mightBorrow, resolveBorrow } = await load()

    expect(await resolveBorrow('broken')).toBeUndefined()
    expect(await resolveBorrow('broken')).toBeUndefined()
    expect(probeSdkConnector).toHaveBeenCalledTimes(1)
    // Asked and answered: no reason to suspend a later spawn over it.
    expect(mightBorrow('broken')).toBe(false)
  })
})

describe('a connection that names no packaged connector', () => {
  it('borrows nothing and waits for nothing', async () => {
    const { mightBorrow, resolveBorrow } = await load()
    expect(mightBorrow('')).toBe(false)
    expect(await resolveBorrow('')).toBeUndefined()
  })
})
