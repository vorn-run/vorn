// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { TerminalSession } from '@vornrun/shared/types'

const resumeMocks = vi.hoisted(() => ({
  resumeEndedSession: vi.fn().mockResolvedValue(undefined),
  dismissEndedSession: vi.fn().mockResolvedValue(undefined)
}))
vi.mock('../src/renderer/lib/session-resume', () => resumeMocks)

const intentBar = vi.hoisted(() => ({
  IntentBar: ({ terminalId }: { terminalId: string }) => (
    <div data-testid="intent-bar">{terminalId}</div>
  )
}))
vi.mock('../src/renderer/components/IntentBar', () => intentBar)

vi.mock('../src/renderer/lib/command-blocks', () => ({
  shortenCwd: (c: string | null) => c
}))

import { SessionComposer } from '../src/renderer/components/card/SessionComposer'
import { useAppStore } from '../src/renderer/stores'
import type { EndedSession } from '../src/renderer/stores/types'

/**
 * What a pane offers when nothing is running behind it.
 *
 * The strip stands where the composer does and replaces it, because a session
 * with no process takes no input -- a composer there would be a control that
 * silently does nothing. That slot is also the only one every surface already
 * has, which is what carries this into tab mode, where there is no card header
 * at all to hang a notice on.
 */

const ID = 'a-session'

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: ID,
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'idle',
    createdAt: Date.now(),
    pid: 1,
    ...over
  } as TerminalSession
}

const ENDED: EndedSession = {
  reason: 'server-stopped',
  at: Date.now() - 3 * 60 * 60 * 1000,
  replayed: true
}

function seed(s: TerminalSession, ended?: EndedSession): void {
  useAppStore.setState({
    terminals: new Map([
      [
        s.id,
        {
          id: s.id,
          session: s,
          status: s.status,
          lastOutputTimestamp: ended?.at ?? Date.now(),
          ...(ended && { ended })
        }
      ]
    ])
  })
}

beforeEach(() => {
  resumeMocks.resumeEndedSession.mockClear()
  resumeMocks.dismissEndedSession.mockClear()
})
afterEach(cleanup)

describe('a pane with nothing running behind it', () => {
  it('says so, instead of offering somewhere to type', () => {
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByText('Ended')).toBeInTheDocument()
    expect(screen.queryByTestId('intent-bar')).not.toBeInTheDocument()
  })

  it.each([
    ['a grid card', { compact: true }],
    ['a tab body', {}],
    ['the focused pane', { indentPx: 12 }]
  ])('appears in %s', (_label, props) => {
    // Every surface renders this one row, which is the reason the strip lives
    // here rather than in the card header -- tab mode has no header.
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} {...props} />)
    expect(screen.getByText('Ended')).toBeInTheDocument()
  })

  it('appears on mobile, where the composer is suppressed', () => {
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} hideIntentBar />)

    expect(screen.getByText('Ended')).toBeInTheDocument()
  })

  it('says what happened and roughly when', () => {
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByText(/the server stopped/)).toBeInTheDocument()
    expect(screen.getByText(/3h ago/)).toBeInTheDocument()
  })

  it('admits when what is on screen stops short', () => {
    seed(session(), { ...ENDED, partial: true })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByText(/the last moments were not recorded/)).toBeInTheDocument()
  })

  it('names the exit code when a shell ended on its own', () => {
    seed(session({ agentType: 'shell' }), {
      reason: 'exited',
      at: Date.now(),
      replayed: true,
      exitCode: 130
    })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByText(/exited with 130/)).toBeInTheDocument()
  })
})

describe('what it offers', () => {
  it('offers to resume an agent, and does not do it on its own', async () => {
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} />)

    const button = screen.getByRole('button', { name: /resume/i })
    expect(resumeMocks.resumeEndedSession).not.toHaveBeenCalled()

    button.click()
    await waitFor(() => expect(resumeMocks.resumeEndedSession).toHaveBeenCalledWith(ID))
  })

  it('offers a shell a fresh one instead, because there is nothing to resume', () => {
    seed(session({ agentType: 'shell', shellCwd: '~/dev/vorn' }), { ...ENDED, cwd: '~/dev/vorn' })
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByRole('button', { name: /new shell here/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^resume$/i })).not.toBeInTheDocument()
    expect(screen.getByText(/~\/dev\/vorn/)).toBeInTheDocument()
  })

  it('can be declined', () => {
    seed(session(), ENDED)
    render(<SessionComposer terminalId={ID} />)

    screen.getByRole('button', { name: /close this pane/i }).click()
    expect(resumeMocks.dismissEndedSession).toHaveBeenCalledWith(ID)
  })
})

describe('a pane with a process behind it', () => {
  it('gets its composer, unchanged', () => {
    seed(session())
    render(<SessionComposer terminalId={ID} />)

    expect(screen.getByTestId('intent-bar')).toBeInTheDocument()
    expect(screen.queryByText('Ended')).not.toBeInTheDocument()
  })

  it('gets nothing at all where the composer is suppressed', () => {
    seed(session())
    const { container } = render(<SessionComposer terminalId={ID} hideIntentBar />)

    expect(container).toBeEmptyDOMElement()
  })
})

describe('the accent', () => {
  it('is not spent on this', () => {
    // Bronzo means a live piece of work is blocked on a person. A crash ends
    // every pane at once, so using it here would put the accent across the whole
    // screen and it would stop meaning anything anywhere.
    seed(session(), ENDED)
    const { container } = render(<SessionComposer terminalId={ID} />)

    expect(container.innerHTML).not.toContain('bronzo')
  })
})
