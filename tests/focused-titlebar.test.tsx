// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, act } from '@testing-library/react'
import type { ReactNode } from 'react'

Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})
Object.defineProperty(window, 'api', {
  value: {
    notifyWidgetStatus: vi.fn(),
    listDir: vi.fn().mockResolvedValue([]),
    killTerminal: vi.fn(),
    listBranches: vi.fn().mockResolvedValue({ local: [], remote: [] })
  },
  writable: true,
  configurable: true
})

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/ConfirmPopover', () => ({
  ConfirmPopover: ({ children }: { children: ReactNode }) => <>{children}</>
}))
vi.mock('../src/renderer/components/GitChangesIndicator', () => ({
  GitChangesIndicator: () => <div data-testid="git-changes" />,
  BrowseFilesButton: () => <div data-testid="browse-files" />
}))
vi.mock('../src/renderer/components/PromotedPaneCard', () => ({
  PromotedPaneCard: ({ cardId }: { cardId: string }) => <div data-testid={`body-${cardId}`} />
}))
vi.mock('../src/renderer/lib/terminal-close', () => ({
  closeTerminalSession: vi.fn().mockResolvedValue(undefined)
}))
vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

// jsdom reports no platform, so the real `isMac` is false and the window never
// hands its titlebar over — which would make every assertion here vacuous.
// Getters rather than fixed values so one file can walk all four platforms.
const platform = { isMac: true, isWeb: false }
vi.mock('../src/renderer/lib/platform', async (orig) => {
  const actual = await orig<typeof import('../src/renderer/lib/platform')>()
  return {
    ...actual,
    get isMac() {
      return platform.isMac
    },
    get isWeb() {
      return platform.isWeb
    }
  }
})

const { useAppStore } = await import('../src/renderer/stores')
const { CardHeader } = await import('../src/renderer/components/card/CardHeader')
const { FocusedStage } = await import('../src/renderer/components/FocusedStage')

const session = {
  id: 't1',
  agentType: 'claude',
  projectName: 'vorn',
  projectPath: '/repo',
  status: 'running',
  createdAt: 0,
  displayName: 'Improve loading',
  branch: 'main',
  isWorktree: false
}

const initialState = useAppStore.getState()

/** The stage as it looks with one session expanded and the sidebar hidden. */
function focusSession(overrides: Record<string, unknown> = {}): void {
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        ['t1', { id: 't1', session, status: 'running', lastOutputTimestamp: 1 }]
      ]),
      focusedTerminalId: 't1',
      previewTerminalId: null,
      isSidebarOpen: false,
      ...overrides
    } as never)
  })
}

/** The same, with a popped-out file on the stage instead of the session. */
function focusCard(overrides: Record<string, unknown> = {}): string {
  focusSession(overrides)
  let cardId = ''
  act(() => {
    cardId = useAppStore.getState().promoteFile('t1', '/repo/src/server.ts')
  })
  act(() => useAppStore.setState({ focusedTerminalId: cardId } as never))
  return cardId
}

/**
 * Every control the hidden titlebar was carrying, asserted one by one.
 *
 * An `&&` of the four collapsed to a single boolean, and `expect(that).toBe(
 * false)` is satisfied by any one of them going missing -- so a header that
 * drew three of the four would have passed as "draws none".
 */
const NAV_CONTROLS = ['Toggle sidebar', 'Sessions', 'Tasks', 'Workflows']

const expectAppNav = (): void => {
  for (const name of NAV_CONTROLS) {
    expect(screen.queryByRole('button', { name })).toBeInTheDocument()
  }
}

const expectNoAppNav = (): void => {
  for (const name of NAV_CONTROLS) {
    expect(screen.queryByRole('button', { name })).not.toBeInTheDocument()
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  platform.isMac = true
  platform.isWeb = false
  act(() => useAppStore.setState(initialState))
})

/**
 * On macOS the app drops its own titlebar while the focus stage fills the
 * window, which leaves the stage's header as the only bar the window has. It
 * was drawing neither the traffic-light inset nor the controls the hidden bar
 * carried, so the lights landed on the session's name and there was no way to
 * reach tasks or workflows without collapsing the session first.
 */
describe('the focused session header, standing in for the titlebar', () => {
  it('clears the traffic lights and carries the app nav when the sidebar is hidden', () => {
    focusSession()
    render(<CardHeader terminalId="t1" variant="focused" />)

    expect(screen.getByTestId('focused-session-header')).toHaveStyle({ paddingLeft: '80px' })
    expectAppNav()
  })

  it('draws neither while the sidebar is open, which is already holding both', () => {
    focusSession({ isSidebarOpen: true })
    render(<CardHeader terminalId="t1" variant="focused" />)

    expect(screen.getByTestId('focused-session-header')).not.toHaveStyle({ paddingLeft: '80px' })
    expectNoAppNav()
  })

  it('draws neither off macOS, where the app keeps its own titlebar', () => {
    platform.isMac = false
    focusSession()
    render(<CardHeader terminalId="t1" variant="focused" />)

    expect(screen.getByTestId('focused-session-header')).not.toHaveStyle({ paddingLeft: '80px' })
    expectNoAppNav()
  })

  it('keeps the nav but drops the inset on the web, which has no traffic lights', () => {
    platform.isWeb = true
    focusSession()
    render(<CardHeader terminalId="t1" variant="focused" />)

    expect(screen.getByTestId('focused-session-header')).not.toHaveStyle({ paddingLeft: '80px' })
    expectAppNav()
  })

  it('leaves the grid’s own card headers alone', () => {
    focusSession()
    render(<CardHeader terminalId="t1" variant="mini" />)

    expect(screen.queryByTestId('focused-session-header')).not.toBeInTheDocument()
    expectNoAppNav()
  })
})

/**
 * A promoted card empties the window of its titlebar exactly as a session does,
 * and draws its own header rather than CardHeader's — so it owes the window the
 * same things, and had the same two gaps.
 */
describe('a promoted card, standing in for the titlebar', () => {
  it('clears the traffic lights and carries the app nav when the sidebar is hidden', () => {
    const cardId = focusCard()
    render(<FocusedStage />)

    expect(screen.getByTestId(`focused-card-${cardId}`)).toHaveStyle({ paddingLeft: '80px' })
    expectAppNav()
  })

  it('draws neither while the sidebar is open', () => {
    const cardId = focusCard({ isSidebarOpen: true })
    render(<FocusedStage />)

    expect(screen.getByTestId(`focused-card-${cardId}`)).not.toHaveStyle({ paddingLeft: '80px' })
    expectNoAppNav()
  })
})
