// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { RestoredSession, TerminalSession } from '../packages/shared/src/types'

const listActiveSessions = vi.fn()
const getRestoredSessions = vi.fn()
const resumeSession = vi.fn()
Object.defineProperty(window, 'api', {
  value: {
    listActiveSessions,
    getRestoredSessions,
    resumeSession,
    notifyWidgetStatus: vi.fn()
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { syncBoard } from '../src/renderer/lib/board-sync'
import { showEndedSession } from '../src/renderer/lib/session-resume'

/**
 * Making the board agree with what the server actually has.
 *
 * The case this exists for is a server dying under a running app. A pane's
 * content lives in the renderer, so nothing about it changes: the terminal keeps
 * showing what it was showing, the bridge reconnects to a replacement holding
 * none of the old PTYs, and the pane goes on accepting input for a process that
 * is gone. It looks exactly like a terminal that has been quiet for a moment,
 * which is the one thing it must never look like when it is a photograph.
 */

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'a-session',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'running',
    createdAt: 1,
    pid: 1,
    ...over
  } as TerminalSession
}

function held(id: string, over: Partial<RestoredSession> = {}): RestoredSession {
  return {
    session: session({ id }),
    endedAt: 1_700_000_000_000,
    replayable: true,
    partial: false,
    closedCleanly: false,
    ...over
  }
}

beforeEach(() => {
  useAppStore.setState({ terminals: new Map(), terminalOrder: [] })
  vi.clearAllMocks()
  live.length = 0
  listActiveSessions.mockImplementation(async () => [...live])
  getRestoredSessions.mockResolvedValue([])
  // As the server does: a resumed session keeps its id -- that is what makes it
  // the same pane -- and is running by the time the call returns, so a later
  // reconciliation sees it live rather than ended.
  resumeSession.mockImplementation(async ({ id }: { id: string }) => {
    const back = session({ id })
    live.push(back)
    return { ok: true, session: back }
  })
})

/** What the server currently has running, which resuming adds to. */
const live: TerminalSession[] = []

/** The ids `sessions:resume` was asked for, in the order it was asked. */
const resumed = (): string[] => resumeSession.mock.calls.map((c) => c[0].id)

const ended = (id: string) => useAppStore.getState().terminals.get(id)?.ended

describe('a server that died under a running app', () => {
  it('marks the panes it was holding, which nothing else would', async () => {
    useAppStore.getState().addTerminal(session({ id: 'was-running' }))
    // The replacement holds nothing, and the record was never saved as restored
    // because the crash ran nothing.
    listActiveSessions.mockResolvedValue([])
    getRestoredSessions.mockResolvedValue([])

    await syncBoard({ showCold: true, resume: false })

    expect(ended('was-running')).toMatchObject({ reason: 'server-stopped', replayed: false })
  })

  it('says a quit was a quit, when the replacement knows that', async () => {
    useAppStore.getState().addTerminal(session({ id: 'one' }))
    getRestoredSessions.mockResolvedValue([held('one', { closedCleanly: true })])

    await syncBoard({ showCold: true, resume: false })

    expect(ended('one')).toMatchObject({ reason: 'app-closed', replayed: true })
  })

  it('leaves a pane alone while its process is still there', async () => {
    useAppStore.getState().addTerminal(session({ id: 'alive' }))
    listActiveSessions.mockResolvedValue([session({ id: 'alive' })])

    await syncBoard({ showCold: true, resume: false })

    expect(ended('alive')).toBeUndefined()
  })

  it('does not overwrite what a pane already said about itself', async () => {
    // A terminal that exited while somebody watched already carries its own
    // reason and its exit code. A later sync must not flatten that into
    // "the server stopped".
    useAppStore.getState().addTerminal(session({ id: 'exited' }), {
      reason: 'exited',
      at: 123,
      replayed: false,
      exitCode: 1
    })

    await syncBoard({ showCold: true, resume: false })

    expect(ended('exited')).toMatchObject({ reason: 'exited', exitCode: 1 })
  })
})

describe('what the server has that this board does not', () => {
  it('gets a pane, however it came to exist', async () => {
    // Started from a phone, or by a workflow, or before this window opened.
    listActiveSessions.mockResolvedValue([session({ id: 'elsewhere' })])

    await syncBoard({ showCold: true, resume: false })

    expect(useAppStore.getState().terminals.has('elsewhere')).toBe(true)
    expect(ended('elsewhere')).toBeUndefined()
  })

  it('shows one that ended before this window opened', async () => {
    getRestoredSessions.mockResolvedValue([held('from-last-run')])

    await syncBoard({ showCold: true, resume: false })

    expect(ended('from-last-run')).toMatchObject({ reason: 'server-stopped', replayed: true })
  })

  it('keeps it off the board when the setting says so', async () => {
    getRestoredSessions.mockResolvedValue([held('from-last-run')])

    await syncBoard({ showCold: false, resume: false })

    expect(useAppStore.getState().terminals.has('from-last-run')).toBe(false)
  })
})

describe('a server that would not answer', () => {
  it('leaves the board as it is rather than emptying it', async () => {
    // Rebuilding from an answer nobody gave would mark every live pane ended.
    useAppStore.getState().addTerminal(session({ id: 'alive' }))
    listActiveSessions.mockRejectedValue(new Error('socket closed'))
    const quiet = vi.spyOn(console, 'error').mockImplementation(() => {})

    await syncBoard({ showCold: true, resume: false })
    quiet.mockRestore()

    expect(ended('alive')).toBeUndefined()
    expect(useAppStore.getState().terminals.has('alive')).toBe(true)
  })
})

describe('starting again what was stopped', () => {
  it('resumes a session whose server died, because that is what the setting means', async () => {
    // "Reopen Sessions on Startup" meant this before the pane ever had a strip:
    // the sessions come back, not just pictures of them.
    getRestoredSessions.mockResolvedValue([held('stopped')])

    await syncBoard({ showCold: true, resume: true })

    expect(resumed()).toEqual(['stopped'])
  })

  it('never resumes one that exited on its own', async () => {
    // An agent that finished its turn chose to end. Starting it again is a
    // surprise the first time and a loop every time after.
    useAppStore.getState().addTerminal(session({ id: 'finished' }), {
      reason: 'exited',
      at: 123,
      replayed: false,
      exitCode: 0
    })

    await syncBoard({ showCold: true, resume: true })

    expect(resumed()).toEqual([])
  })

  it('starts nothing when the setting is off', async () => {
    getRestoredSessions.mockResolvedValue([held('stopped')])

    await syncBoard({ showCold: true, resume: false })

    expect(resumed()).toEqual([])
    expect(ended('stopped')).toBeDefined()
  })

  it('leaves alone a pane that was already sitting there ended', async () => {
    // The setting is on and the server is replaced again -- a second crash, or a
    // relaunch that took two attempts. A pane somebody looked at and chose not
    // to resume must not be resumed by the next reconciliation that runs.
    useAppStore.getState().addTerminal(session({ id: 'left-alone' }), {
      reason: 'server-stopped',
      at: 123,
      replayed: true
    })

    await syncBoard({ showCold: true, resume: true })

    expect(resumed()).toEqual([])
  })

  it('does not resume a session that is still running', async () => {
    listActiveSessions.mockResolvedValue([session({ id: 'alive' })])
    useAppStore.getState().addTerminal(session({ id: 'alive' }))

    await syncBoard({ showCold: true, resume: true })

    expect(resumed()).toEqual([])
  })
})

describe('two reconciliations at once', () => {
  it('resumes each session once, however many passes overlap', async () => {
    // Start-up runs one and a replaced server runs another moments later; in
    // development the mount effect runs twice on its own. Both passes see the
    // same cold records. Before they were serialised, the loser was told the
    // session was gone and deleted the pane the winner had just brought back --
    // which is how a board ended up with one pane for two live sessions.
    getRestoredSessions.mockResolvedValue([held('one'), held('two')])
    // The second pass asks the server what is running before the first pass has
    // resumed anything, and is answered after it has. Its answer is therefore
    // already stale when it is used -- which is the whole race, and why running
    // the passes one after another is the only thing that settles it.
    let answer: () => void = () => {}
    const held_back = new Promise<void>((r) => (answer = r))
    listActiveSessions
      .mockImplementationOnce(async () => [...live])
      .mockImplementationOnce(async () => {
        const snapshot = [...live]
        await held_back
        return snapshot
      })

    const first = syncBoard({ showCold: true, resume: true })
    const second = syncBoard({ showCold: true, resume: true })
    await first
    answer()
    await second

    expect(resumed().sort()).toEqual(['one', 'two'])
  })

  it('keeps the pane when an automatic resume loses the claim', async () => {
    // The claim is almost always lost to this same app, and an automatic resume
    // says nothing when it fails -- so removing the pane here made one vanish
    // with no explanation anywhere.
    getRestoredSessions.mockResolvedValue([held('contested')])
    resumeSession.mockResolvedValue({ ok: false, reason: 'gone' })

    await syncBoard({ showCold: true, resume: true })

    expect(useAppStore.getState().terminals.has('contested')).toBe(true)
  })
})

describe('bringing an ended pane in from the banner', () => {
  it('says a quit was a quit, as the board does', async () => {
    // The banner offers to show panes without starting anything, and the record
    // it reads says which ending this was. Reporting every one of them as a
    // server that stopped unexpectedly turns closing the app into a fault report.
    getRestoredSessions.mockResolvedValue([held('quit', { closedCleanly: true })])

    await showEndedSession('quit')

    expect(ended('quit')).toMatchObject({ reason: 'app-closed' })
  })

  it('still says so when the server really did stop', async () => {
    getRestoredSessions.mockResolvedValue([held('crashed', { closedCleanly: false })])

    await showEndedSession('crashed')

    expect(ended('crashed')).toMatchObject({ reason: 'server-stopped' })
  })
})
