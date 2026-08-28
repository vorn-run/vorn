// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const registryMocks = vi.hoisted(() => ({
  pasteToTerminal: vi.fn(),
  focusTerminal: vi.fn(),
  scrollToBottom: vi.fn(),
  registerSlot: vi.fn(),
  unregisterSlot: vi.fn(),
  hydrateTerminal: vi.fn(),
  registerStatusHandler: vi.fn().mockReturnValue(() => {}),
  onTerminalReady: vi.fn().mockReturnValue(() => {})
}))
vi.mock('../src/renderer/lib/terminal-registry', () => registryMocks)

const resumeMocks = vi.hoisted(() => ({
  resumeEndedSession: vi.fn().mockResolvedValue(undefined),
  dismissEndedSession: vi.fn().mockResolvedValue(undefined)
}))
vi.mock('../src/renderer/lib/session-resume', () => resumeMocks)

// The composer's other branch pulls in the whole intent bar; this file is about
// which branch is taken, not what the bar does.
vi.mock('../src/renderer/components/IntentBar', () => ({
  IntentBar: ({ terminalId }: { terminalId: string }) => (
    <div data-testid="intent-bar">{terminalId}</div>
  )
}))

Object.defineProperty(window, 'api', {
  value: {
    notifyWidgetStatus: vi.fn(),
    killTerminal: vi.fn().mockResolvedValue(undefined)
  },
  writable: true
})

import type { TerminalSession } from '../packages/shared/src/types'
import { useAppStore } from '../src/renderer/stores'
import { SessionComposer } from '../src/renderer/components/card/SessionComposer'

/**
 * The row beneath a terminal, when there is nothing behind the terminal.
 *
 * Two things are being pinned. That a pane with no process shows what happened
 * instead of a place to type -- a composer there is a control that silently
 * does nothing. And that it does so on every surface, including tab mode, which
 * renders no card header at all and is where a per-call-site condition would
 * have been forgotten.
 */

const ID = 'a-session'

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: ID,
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'idle',
    createdAt: 1,
    pid: 1,
    ...over
  } as TerminalSession
}

beforeEach(() => {
  useAppStore.setState({ terminals: new Map(), terminalOrder: [] })
  vi.clearAllMocks()
})
afterEach(cleanup)

describe('which row a pane gets', () => {
  it('offers a place to type while something is running', () => {
    useAppStore.getState().addTerminal(session())
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByTestId('intent-bar')).toBeInTheDocument()
    expect(screen.queryByText('Ended')).not.toBeInTheDocument()
  })

  it('says what happened instead, once nothing is', () => {
    useAppStore.getState().addTerminal(session(), {
      reason: 'server-stopped',
      at: Date.now() - 3_600_000,
      replayed: true
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByText('Ended')).toBeInTheDocument()
    // The composer is gone, not merely disabled. A session with no process
    // takes no input, and a box that swallows typing is worse than none.
    expect(screen.queryByTestId('intent-bar')).not.toBeInTheDocument()
  })

  it.each([
    ['a grid card', { compact: true }],
    ['a tab body', {}],
    ['the focused pane', { indentPx: 12 }]
  ])('says it on %s', (_label, props) => {
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })
    render(<SessionComposer terminalId={ID} {...props} />)

    expect(screen.getByText('Ended')).toBeInTheDocument()
  })

  it('still says it where the composer is suppressed', () => {
    // The focused pane hides its intent bar on mobile, where the keyboard owns
    // that space. A pane whose session ended still has something to say there.
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })
    render(<SessionComposer terminalId={ID} hideIntentBar />)

    expect(screen.getByText('Ended')).toBeInTheDocument()
  })

  it('shows nothing at all where the composer is suppressed and the session is live', () => {
    useAppStore.getState().addTerminal(session())
    const { container } = render(<SessionComposer terminalId={ID} hideIntentBar />)

    expect(container).toBeEmptyDOMElement()
  })
})

describe('what the strip says', () => {
  it('names the cause and how long ago', () => {
    useAppStore.getState().addTerminal(session(), {
      reason: 'server-stopped',
      at: Date.now() - 3 * 3_600_000,
      replayed: true
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByRole('status')).toHaveTextContent('the server stopped')
    expect(screen.getByRole('status')).toHaveTextContent('3h ago')
  })

  it('names an exit code when there was one', () => {
    useAppStore.getState().addTerminal(session({ agentType: 'shell' }), {
      reason: 'exited',
      at: Date.now(),
      replayed: true,
      exitCode: 1
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByRole('status')).toHaveTextContent('exited with 1')
  })

  it('admits when what is on screen is not the whole story', () => {
    useAppStore.getState().addTerminal(session(), {
      reason: 'server-stopped',
      at: Date.now(),
      replayed: true,
      partial: true
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByRole('status')).toHaveTextContent('the last moments were not recorded')
  })

  it('offers a shell a new one rather than a resume it cannot do', () => {
    useAppStore.getState().addTerminal(session({ agentType: 'shell' }), {
      reason: 'exited',
      at: Date.now(),
      replayed: true,
      cwd: '/Users/x/dev/vorn'
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByRole('button', { name: /new shell here/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^resume$/i })).not.toBeInTheDocument()
  })
})

describe('the colour it does not use', () => {
  it('never takes the accent', () => {
    // Bronzo means a live piece of work is blocked on a person. A crash ends
    // every pane at once, so the accent would land on the whole screen and stop
    // meaning anything anywhere.
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })
    const { container } = render(<SessionComposer terminalId={ID} />)

    expect(container.innerHTML).not.toMatch(/bronzo/)
  })
})

describe('acting on the offer', () => {
  it('resumes through the server, which hands the record out once', async () => {
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })
    render(<SessionComposer terminalId={ID} />)

    fireEvent.click(screen.getByRole('button', { name: /resume/i }))

    await waitFor(() => expect(resumeMocks.resumeEndedSession).toHaveBeenCalledWith(ID))
  })

  it('lets it go through the same door', async () => {
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })
    render(<SessionComposer terminalId={ID} />)

    fireEvent.click(screen.getByRole('button', { name: /close this pane/i }))

    await waitFor(() => expect(resumeMocks.dismissEndedSession).toHaveBeenCalledWith(ID))
  })
})

describe('resuming keeps the card where it was', () => {
  it("puts the new session in the old one's place, not at the end", () => {
    // Resuming spawns a session with a new id. Appending it would read as a
    // pane that vanished and a different one that arrived somewhere else.
    const store = useAppStore.getState()
    store.addTerminal(session({ id: 'first' }))
    store.addTerminal(session({ id: 'ended' }), {
      reason: 'server-stopped',
      at: Date.now(),
      replayed: true
    })
    store.addTerminal(session({ id: 'last' }))

    useAppStore.getState().replaceTerminal('ended', session({ id: 'resumed' }))

    expect(useAppStore.getState().terminalOrder).toEqual(['first', 'resumed', 'last'])
  })

  it('leaves nothing of the session it replaced', () => {
    const store = useAppStore.getState()
    store.addTerminal(session({ id: 'ended' }), {
      reason: 'server-stopped',
      at: Date.now(),
      replayed: true
    })

    useAppStore.getState().replaceTerminal('ended', session({ id: 'resumed' }))
    const state = useAppStore.getState()

    expect(state.terminals.has('ended')).toBe(false)
    // And the replacement is live: no strip on a pane with a process behind it.
    expect(state.terminals.get('resumed')?.ended).toBeUndefined()
  })
})

describe('when a pane came back from a previous run', () => {
  it('is not stamped with the time it was restored', () => {
    // Several views sort by last output. Stamping every restored pane with the
    // current time puts a terminal nobody has touched in days at the top.
    const endedAt = Date.now() - 5 * 3_600_000
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: endedAt, replayed: true })

    expect(useAppStore.getState().terminals.get(ID)?.lastOutputTimestamp).toBe(endedAt)
  })

  it('is never stamped zero, which would draw the spawning skeleton over it', () => {
    // `lastOutputTimestamp === 0` is what the card and the tab body use to show
    // a pane that has not produced anything yet. A replayed screen underneath
    // that shimmer would be the wrong story twice over.
    useAppStore
      .getState()
      .addTerminal(session(), { reason: 'server-stopped', at: Date.now(), replayed: true })

    expect(useAppStore.getState().terminals.get(ID)?.lastOutputTimestamp).not.toBe(0)
  })
})
