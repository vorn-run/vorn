import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * Every change the pty manager makes to a terminal's record is told to vornd's
 * copy of the registry, and nothing about an extension's pane is.
 */

vi.mock('../packages/server/src/vornd-sessions', async () =>
  (await import('./helpers/fake-vornd-pty')).vorndModule()
)
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: vi.fn(() => ({ defaults: { shell: '/bin/sh' } })) }
}))
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => null),
  getGitHead: vi.fn(async () => null),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn(() => 'wt'),
  isGitRepo: vi.fn(async () => false)
}))

const { ptyManager } = await import('../packages/server/src/pty-manager')
const { sessionFeed } = await import('../packages/server/src/session-feed')
const { fakeVornd } = await import('./helpers/fake-vornd-pty')

let sent: Array<Record<string, unknown>>
const opened: string[] = []

beforeEach(() => {
  fakeVornd.reset()
  sent = []
  sessionFeed.attach({ wants: () => true, send: (p) => sent.push(structuredClone(p)) })
})

afterEach(() => {
  for (const id of opened.splice(0)) ptyManager.killPty(id)
  sessionFeed.attach(null)
})

function upserts(id: string): Array<Record<string, unknown>> {
  return sent.filter(
    (s) => s.op === 'upsert' && (s.record as { id?: string } | undefined)?.id === id
  )
}

function lastRecord(id: string): Record<string, unknown> {
  return upserts(id).at(-1)?.record as Record<string, unknown>
}

describe('the records vornd is told', () => {
  it('follows a shell from its start to its exit', () => {
    const shell = ptyManager.createShellPty('/tmp')
    opened.push(shell.id)
    const program = fakeVornd.last()
    expect(lastRecord(shell.id)).toMatchObject({ id: shell.id, status: 'running', pid: 0 })
    expect(sent.filter((s) => s.op === 'order').at(-1)?.order).toContain(shell.id)

    program.start(4242)
    expect(lastRecord(shell.id).pid).toBe(4242)
    program.cwd('/var')
    expect(lastRecord(shell.id).shellCwd).toBe('/var')
    ptyManager.resizePty(shell.id, 120, 40)
    expect(lastRecord(shell.id)).toMatchObject({ cols: 120, rows: 40 })
    ptyManager.renameSession(shell.id, 'Build')
    ptyManager.setSessionGroup(shell.id, 'g')
    expect(lastRecord(shell.id)).toMatchObject({ displayName: 'Build', groupId: 'g' })
    ptyManager.setSessionGroup(shell.id, null)
    expect(lastRecord(shell.id)).not.toHaveProperty('groupId')

    // Nothing changed: nothing sent.
    const before = sent.length
    ptyManager.resizePty(shell.id, 120, 40)
    ptyManager.recordChanged(shell.id)
    expect(sent).toHaveLength(before)

    program.exitAt = { epoch: 1, rseq: 30, index: 0 }
    program.exit(2)
    const exit = upserts(shell.id).at(-1)!
    expect(exit).toMatchObject({
      record: { status: 'idle', shellExitCode: 2 },
      exitAt: { epoch: 1, rseq: 30, index: 0 }
    })
    expect(sent.filter((s) => s.op === 'order').at(-1)?.order).not.toContain(shell.id)

    ptyManager.killPty(shell.id)
    opened.pop()
    expect(sent.at(-1)).toEqual({ op: 'remove', kind: 'terminal', id: shell.id })
  })

  it('stamps a status vornd told with its record, and not one from input', async () => {
    const agent = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)
    opened.push(agent.id)
    const program = fakeVornd.last()
    program.status(2, { epoch: 3, rseq: 7, index: 1 })
    expect(upserts(agent.id).at(-1)).toMatchObject({
      record: { status: 'waiting' },
      statusAt: { epoch: 3, rseq: 7, index: 1 }
    })
    ptyManager.writeToPty(agent.id, 'y')
    const typed = upserts(agent.id).at(-1)!
    expect(typed).toMatchObject({ record: { status: 'running' } })
    expect(typed).not.toHaveProperty('statusAt')

    ptyManager.promoteToHookStatus(agent.id)
    expect(lastRecord(agent.id).statusSource).toBe('hooks')
  })

  it('tells the order and a resume, and nothing of an extension pane', () => {
    const a = ptyManager.createShellPty('/tmp')
    const b = ptyManager.createShellPty('/tmp')
    opened.push(a.id, b.id)
    ptyManager.reorderSessions([b.id, a.id])
    expect(sent.at(-1)).toEqual({ op: 'order', order: [b.id, a.id] })

    ptyManager.releaseForResume(a.id)
    opened.splice(opened.indexOf(a.id), 1)
    expect(sent.slice(-2)).toEqual([
      { op: 'remove', kind: 'terminal', id: a.id },
      { op: 'order', order: [b.id] }
    ])

    const told = sent.length
    const pane = ptyManager.createExtensionPty({
      command: 'lazygit',
      args: [],
      cwd: '/tmp',
      displayName: 'Git',
      env: {}
    })
    fakeVornd.last().start(99)
    ptyManager.resizePty(pane.id, 90, 30)
    ptyManager.recordChanged(pane.id)
    ptyManager.killPty(pane.id)
    expect(sent.slice(told).filter((s) => s.op !== 'order')).toEqual([])
  })
})
