import { afterEach, describe, expect, it, vi } from 'vitest'

/**
 * Where a pane's pages are served from.
 *
 * Not the app's origin. A page and the web client sharing one would share a
 * browser's storage and be able to open each other's sockets, so a page that
 * loaded something hostile would hold whatever the client holds and the
 * permission list would mean nothing. Its own port costs one listener.
 */

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

vi.mock('../packages/server/src/connectors/packs', async (importOriginal) => ({
  WEB_FILE_TYPES: (await importOriginal<typeof import('../packages/server/src/connectors/packs')>())
    .WEB_FILE_TYPES,
  installedPack: () => undefined
}))

vi.mock('../packages/server/src/extensions/panes', () => ({ grantFor: () => undefined }))
vi.mock('../packages/server/src/extensions/hosts', () => ({ hostByToken: () => undefined }))
vi.mock('../packages/server/src/extensions/selection', () => ({ requestSelection: async () => '' }))
vi.mock('../packages/server/src/extensions/usage', () => ({ usageFor: () => ({}) }))
vi.mock('../packages/server/src/git-utils', () => ({
  getGitDiffText: () => '',
  getGitStatusPorcelain: () => ''
}))
vi.mock('../packages/server/src/pty-manager', () => ({ ptyManager: { getLiveSessions: () => [] } }))

const { startExtensionPageServer, stopExtensionPageServer, extensionPageOrigin } =
  await import('../packages/server/src/extensions/page-server')

afterEach(async () => {
  await stopExtensionPageServer()
})

const deps = { frameAncestors: () => ["'self'"], sessionRenamed: () => {} }

describe('the origin pages are served from', () => {
  it('is loopback, and its own', async () => {
    const server = await startExtensionPageServer(deps)
    expect(server.origin).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
    expect(extensionPageOrigin()).toBe(server.origin)
  })

  // Everything the app is reached through is deliberately absent here.
  it('serves nothing but pages', async () => {
    const server = await startExtensionPageServer(deps)
    const port = Number(new URL(server.origin).port)

    const paths = ['/app/', '/ws', '/health', '/api/pair/poll', '/']
    const answered = await Promise.all(
      paths.map(async (path) => {
        const answer = await fetch(`http://127.0.0.1:${port}${path}`).catch(() => undefined)
        return `${path} ${answer?.status ?? 'refused'}`
      })
    )
    expect(answered).toEqual(paths.map((path) => `${path} 404`))
  })

  it('is started once, however often it is asked for', async () => {
    const first = await startExtensionPageServer(deps)
    const second = await startExtensionPageServer(deps)
    expect(second.origin).toBe(first.origin)
  })

  it('has no origin once it has stopped', async () => {
    await startExtensionPageServer(deps)
    await stopExtensionPageServer()
    expect(extensionPageOrigin()).toBe('')
  })
})
