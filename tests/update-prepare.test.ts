import { describe, it, expect, vi, beforeEach } from 'vitest'

const stopServer = vi.fn(async () => {})
const detachFromServer = vi.fn()
vi.mock('../src/main/server/server-launcher', () => ({ stopServer, detachFromServer }))

const { releaseServerForUpdate } = await import('../src/main/server/update-prepare')

beforeEach(() => {
  stopServer.mockClear()
  detachFromServer.mockClear()
})

describe('what an update does to the server before the installer runs', () => {
  it('stops it on Windows, where the installer cannot replace a running exe', async () => {
    await releaseServerForUpdate('win32')
    expect(stopServer).toHaveBeenCalledTimes(1)
    expect(detachFromServer).not.toHaveBeenCalled()
  })

  it('lets go of it elsewhere, so the next build can take the sessions over', async () => {
    for (const platform of ['darwin', 'linux'] as const) {
      await releaseServerForUpdate(platform)
    }
    expect(detachFromServer).toHaveBeenCalledTimes(2)
    expect(stopServer).not.toHaveBeenCalled()
  })
})
