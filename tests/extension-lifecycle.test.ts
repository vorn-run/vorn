import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'

/**
 * What a session ending releases.
 *
 * An extension is started for the sessions of a project, so the last of those
 * ending is what makes its child pointless. Anything short of that leaves a
 * process per project someone once opened, for the life of the server.
 */

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const live: TerminalSession[] = []
const stoppedFooters: string[] = []
const closedForSession: string[] = []
const closedForTerminal: string[] = []
const stoppedProjects: string[] = []

vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: {
    getLiveSessions: () => live,
    on: vi.fn(),
    getActiveSessions: () => live
  }
}))

vi.mock('../packages/server/src/extensions/footers', () => ({
  stopFooters: (id: string) => stoppedFooters.push(id),
  syncFooters: vi.fn(),
  footerReadings: () => []
}))

vi.mock('../packages/server/src/extensions/panes', () => ({
  closePane: vi.fn(),
  closePaneForTerminal: (id: string) => closedForTerminal.push(id),
  closePanesForExtension: vi.fn(),
  closePanesForSession: (id: string) => closedForSession.push(id),
  openPane: vi.fn()
}))

vi.mock('../packages/server/src/extensions/hosts', () => ({
  installedExtensions: () => [],
  stopHostsForExtension: vi.fn(),
  stopHostsForProject: async (path: string) => {
    stoppedProjects.push(path)
  }
}))

const { releaseExtensionsFor } = await import('../packages/server/src/register-methods')

const session = (over: Partial<TerminalSession> = {}): TerminalSession =>
  ({
    id: 's1',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    status: 'running',
    createdAt: 0,
    pid: 1,
    ...over
  }) as TerminalSession

beforeEach(() => {
  live.length = 0
  stoppedFooters.length = 0
  closedForSession.length = 0
  closedForTerminal.length = 0
  stoppedProjects.length = 0
})

describe('a session ending', () => {
  it('releases its footers and its panes', () => {
    releaseExtensionsFor(session())
    expect(stoppedFooters).toEqual(['s1'])
    expect(closedForSession).toEqual(['s1'])
    // The ended session may itself have been an extension's pane terminal.
    expect(closedForTerminal).toEqual(['s1'])
  })

  it('stops the project child once nothing is left that could ask it anything', () => {
    releaseExtensionsFor(session())
    expect(stoppedProjects).toEqual(['/work/vorn'])
  })

  it('keeps the child while another session of that project is running', () => {
    live.push(session({ id: 's2' }))
    releaseExtensionsFor(session())
    expect(stoppedProjects).toEqual([])
  })

  it('stops it when the sessions that are left belong to other projects', () => {
    live.push(session({ id: 's2', projectPath: '/work/other' }))
    releaseExtensionsFor(session())
    expect(stoppedProjects).toEqual(['/work/vorn'])
  })

  // The list is read after the exit, so the session ending can still be in it.
  it('does not count the session that is ending as a reason to keep it', () => {
    live.push(session())
    releaseExtensionsFor(session())
    expect(stoppedProjects).toEqual(['/work/vorn'])
  })
})
