import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InstalledConnectorPack, TerminalSession } from '@vornrun/shared/types'

/**
 * What an open pane entitles, and for how long.
 *
 * A page proves itself with the nonce in its own URL rather than the token its
 * extension holds, so what that nonce is good for — which session, which pane,
 * until when — is the whole of a page's authority.
 */

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const packs = new Map<string, InstalledConnectorPack>()
const started: Array<{ extensionId: string; projectPath: string }> = []
const spawned: Array<{
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
}> = []
const killed: string[] = []

vi.mock('../packages/server/src/connectors/packs', () => ({
  installedPack: (id: string) => packs.get(id)
}))

vi.mock('../packages/server/src/extensions/hosts', () => ({
  getOrStartHost: async (extensionId: string, projectPath: string) => {
    started.push({ extensionId, projectPath })
    return {}
  },
  tokenFor: () => 'the-process-token',
  extensionBridgeOrigin: () => 'http://127.0.0.1:5000'
}))

vi.mock('../packages/server/src/extensions/page-server', () => ({
  extensionPageOrigin: () => 'http://127.0.0.1:6000'
}))

vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: {
    createExtensionPty: (params: {
      command: string
      args: string[]
      cwd: string
      env: Record<string, string>
    }) => {
      spawned.push(params)
      return { id: `pty-${spawned.length}` }
    },
    killPty: (id: string) => killed.push(id)
  }
}))

const panes = await import('../packages/server/src/extensions/panes')

function extension(): InstalledConnectorPack {
  return {
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    path: '/packs/review',
    installedAt: 0,
    bytes: 0,
    triggers: [],
    actions: [],
    env: [],
    contributes: {
      panes: [
        { id: 'report', title: 'Report', web: 'web/report/index.html' },
        { id: 'git', title: 'Git', command: ['lazygit'] }
      ]
    },
    permissions: []
  }
}

const session = (over: Partial<TerminalSession> = {}): TerminalSession =>
  ({
    id: 's1',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    worktreePath: '/work/vorn-tree',
    status: 'running',
    createdAt: 0,
    pid: 1,
    ...over
  }) as TerminalSession

beforeEach(() => {
  packs.clear()
  packs.set('review', extension())
  started.length = 0
  spawned.length = 0
  killed.length = 0
})

afterEach(() => {
  vi.useRealTimers()
})

describe('opening a pane', () => {
  it('draws a page from the origin the pages are served on', async () => {
    const grant = await panes.openPane('review', 'report', session())
    expect(grant.url).toBe(`http://127.0.0.1:6000/extensions/review/pane/report/${grant.nonce}/`)
    expect(started).toEqual([{ extensionId: 'review', projectPath: '/work/vorn' }])
  })

  // A program is the extension's own code, so it holds the token — and needs somewhere to spend it.
  it('gives a program the token and the address that token is good at', async () => {
    await panes.openPane('review', 'git', session())
    expect(spawned).toHaveLength(1)
    expect(spawned[0].cwd).toBe('/work/vorn-tree')
    expect(spawned[0].env).toEqual({
      VORN_EXTENSION_TOKEN: 'the-process-token',
      VORN_EXTENSION_HOST: 'http://127.0.0.1:5000/extensions/review/bridge'
    })
  })

  it('opens no pane the extension does not contribute', async () => {
    await expect(panes.openPane('review', 'nothing', session())).rejects.toThrow(/contributes no/)
  })
})

describe('what a nonce is good for', () => {
  it('names the pane and the session it was opened as', async () => {
    const grant = await panes.openPane('review', 'report', session())
    expect(panes.grantFor(grant.nonce)).toMatchObject({
      extensionId: 'review',
      paneId: 'report',
      sessionId: 's1',
      projectPath: '/work/vorn'
    })
    expect(panes.grantFor('made-up')).toBeUndefined()
  })

  it('stops meaning anything once the pane is closed', async () => {
    const grant = await panes.openPane('review', 'report', session())
    expect(panes.closePane(grant.nonce)).toBe(true)
    expect(panes.grantFor(grant.nonce)).toBeUndefined()
    expect(panes.closePane(grant.nonce)).toBe(false)
  })

  // A pane the app forgot to close should not leave authority lying around.
  it('goes stale when nothing has used it for long enough', async () => {
    vi.useFakeTimers()
    const grant = await panes.openPane('review', 'report', session())

    vi.advanceTimersByTime(11 * 60 * 60 * 1000)
    expect(panes.grantFor(grant.nonce)).toBeDefined()

    // Using it puts the clock back, so a pane in use never expires under its page.
    vi.advanceTimersByTime(11 * 60 * 60 * 1000)
    expect(panes.grantFor(grant.nonce)).toBeDefined()

    vi.advanceTimersByTime(13 * 60 * 60 * 1000)
    expect(panes.grantFor(grant.nonce)).toBeUndefined()
  })

  it('closes a program pane by the terminal that was drawing it', async () => {
    const grant = await panes.openPane('review', 'git', session())
    expect(grant.terminalId).toBe('pty-1')

    panes.closePaneForTerminal('pty-1')
    expect(panes.grantFor(grant.nonce)).toBeUndefined()
    // The program is already gone; killing it again is what we are not doing.
    expect(killed).toEqual([])
  })

  it('closes every pane a session had, and every pane an extension had', async () => {
    const first = await panes.openPane('review', 'report', session())
    const second = await panes.openPane('review', 'report', session({ id: 's2' }))

    panes.closePanesForSession('s1')
    expect(panes.grantFor(first.nonce)).toBeUndefined()
    expect(panes.grantFor(second.nonce)).toBeDefined()

    panes.closePanesForExtension('review')
    expect(panes.grantFor(second.nonce)).toBeUndefined()
  })
})
