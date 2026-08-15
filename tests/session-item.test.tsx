// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { useAppStore } from '../src/renderer/stores'
import { SessionItem } from '../src/renderer/components/project-sidebar/SessionItem'
import type { SidebarSessionInfo } from '../src/renderer/components/project-sidebar/types'

const session: SidebarSessionInfo = {
  id: 'sess-1',
  name: 'My Session',
  status: 'running',
  agentType: 'claude',
  branch: 'main',
  isWorktree: false
}

const initialState = useAppStore.getState()

const MOBILE = { isMobile: true, framework: 'expo' as const, needsDevClient: true }
const WEB = { isMobile: false, framework: null, needsDevClient: false }

/** A session row needs a terminal behind it for the device gate to find a project path. */
function seedProject(mobile?: typeof MOBILE | typeof WEB): void {
  const terminals = new Map()
  terminals.set(session.id, {
    id: session.id,
    session: { id: session.id, projectPath: '/proj', projectName: 'p', agentType: 'claude' },
    status: 'idle',
    lastOutputTimestamp: 1
  })
  act(() => {
    useAppStore.setState({
      terminals,
      devicePanes: new Map(),
      mobileProjectCache: mobile ? new Map([['/proj', mobile]]) : new Map(),
      loadMobileProject: async () => {}
    })
  })
}

describe('SessionItem', () => {
  beforeEach(() => {
    useAppStore.setState({ focusedTerminalId: null })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('renders session name', () => {
    render(<SessionItem session={session} />)
    expect(screen.getByText('My Session')).toBeInTheDocument()
  })

  it('renders branch when showBranch is true (default)', () => {
    render(<SessionItem session={session} />)
    expect(screen.getByText('main')).toBeInTheDocument()
  })

  it('hides branch when showBranch is false', () => {
    render(<SessionItem session={session} showBranch={false} />)
    expect(screen.queryByText('main')).not.toBeInTheDocument()
  })

  it('replaces the agent identity icon with the running glyph when running', () => {
    const { container } = render(<SessionItem session={session} />)
    const glyph = container.querySelector('[data-component="running-glyph"]')
    expect(glyph).toBeInTheDocument()
    expect(glyph).toHaveAttribute('aria-label', 'Running')
  })

  it('calls setFocusedTerminal on click', () => {
    const setFocused = vi.fn()
    useAppStore.setState({ setFocusedTerminal: setFocused })
    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByText('My Session'))
    expect(setFocused).toHaveBeenCalledWith('sess-1')
  })

  it('applies focused style when session is focused', () => {
    useAppStore.setState({ focusedTerminalId: 'sess-1' })
    const { container } = render(<SessionItem session={session} />)
    const button = container.querySelector('button')
    expect(button?.className).toContain('text-white')
  })

  it('applies unfocused style when session is not focused', () => {
    useAppStore.setState({ focusedTerminalId: 'other' })
    const { container } = render(<SessionItem session={session} />)
    const button = container.querySelector('button')
    expect(button?.className).toContain('text-gray-400')
  })

  it('renders without branch when session has no branch', () => {
    const noBranch: SidebarSessionInfo = { ...session, branch: undefined }
    render(<SessionItem session={noBranch} />)
    expect(screen.queryByText('main')).not.toBeInTheDocument()
  })

  it.each(['waiting', 'idle', 'error'] as const)(
    'renders the plain agent identity icon (no pulse) for %s status',
    (status) => {
      const s: SidebarSessionInfo = { ...session, status }
      const { container } = render(<SessionItem session={s} />)
      expect(container.querySelector('[data-component="running-glyph"]')).toBeNull()
      // Identity svg should be rendered inside the icon wrapper, not matched globally
      const sessionButton = screen.getByText('My Session').closest('button')
      const iconWrapper = sessionButton?.querySelector('span')
      expect(iconWrapper?.querySelector('svg')).toBeInTheDocument()
    }
  )

  it('renders close button', () => {
    const { container } = render(<SessionItem session={session} />)
    const closeBtn = container.querySelector('button[type="button"]')
    expect(closeBtn).toBeInTheDocument()
  })

  it("toggles this session's files pane without selecting the session", () => {
    const setActiveTabId = vi.fn()
    act(() => useAppStore.setState({ setActiveTabId }))

    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByRole('button', { name: /Show files for/ }))

    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(true)
    // The row is itself a button; the toggle must stop propagation or opening
    // files would also switch the active session.
    expect(setActiveTabId).not.toHaveBeenCalled()
  })

  it("toggles this session's browser pane without selecting the session", () => {
    const setActiveTabId = vi.fn()
    act(() => useAppStore.setState({ setActiveTabId }))

    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByRole('button', { name: /Show browser for/ }))

    expect(useAppStore.getState().browserPanes.has(session.id)).toBe(true)
    expect(setActiveTabId).not.toHaveBeenCalled()
  })

  it('reflects and clears an open browser pane', () => {
    act(() => useAppStore.getState().openBrowserPane(session.id, 'example.com'))
    render(<SessionItem session={session} />)

    fireEvent.click(screen.getByRole('button', { name: /Hide browser for/ }))
    expect(useAppStore.getState().browserPanes.has(session.id)).toBe(false)
  })

  it('reflects and clears an open files pane', () => {
    act(() => useAppStore.getState().openFilesPane(session.id))
    render(<SessionItem session={session} />)

    const btn = screen.getByRole('button', { name: /Hide files for/ })
    fireEvent.click(btn)
    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(false)
  })
})

describe('SessionItem device control', () => {
  it('offers a device on a mobile project', () => {
    seedProject(MOBILE)
    render(<SessionItem session={session} />)
    expect(screen.getByRole('button', { name: /Show device for/ })).toBeInTheDocument()
  })

  it('stays out of the way on a project with no mobile app in it', () => {
    // A simulator button on a web or backend repo is a control that can only
    // disappoint, sitting next to two that always work.
    seedProject(WEB)
    render(<SessionItem session={session} />)
    expect(screen.queryByRole('button', { name: /device for/ })).not.toBeInTheDocument()
  })

  it('shows nothing while the project is still unprobed', () => {
    seedProject(undefined)
    render(<SessionItem session={session} />)
    expect(screen.queryByRole('button', { name: /device for/ })).not.toBeInTheDocument()
  })

  it('keeps the control while a device pane is open, whatever detection says', () => {
    seedProject(WEB)
    act(() => useAppStore.getState().openDevicePane(session.id, { udid: 'u1', name: 'iPhone 17' }))
    render(<SessionItem session={session} />)
    // Detection is a heuristic and will be wrong eventually. A control that
    // vanishes out from under a device someone is driving leaves them no way to
    // close it.
    expect(screen.getByRole('button', { name: /Hide device for/ })).toBeInTheDocument()
  })

  it('closes an open device pane rather than reopening the picker', () => {
    const deviceRelease = vi.fn().mockResolvedValue({ released: true })
    Object.defineProperty(window, 'api', {
      value: { ...(window as unknown as { api?: object }).api, deviceRelease },
      writable: true,
      configurable: true
    })
    seedProject(MOBILE)
    act(() => useAppStore.getState().openDevicePane(session.id, { udid: 'u1', name: 'iPhone 17' }))
    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByRole('button', { name: /Hide device for/ }))
    expect(useAppStore.getState().devicePanes.has(session.id)).toBe(false)
    expect(deviceRelease).toHaveBeenCalledWith(session.id)
  })
})

describe('SessionItem terminals control', () => {
  const seedApi = (createShellTerminal: unknown): void => {
    Object.defineProperty(window, 'api', {
      value: {
        ...(window as unknown as { api?: object }).api,
        createShellTerminal,
        notifyWidgetStatus: vi.fn(),
        reorderSessions: vi.fn()
      },
      writable: true,
      configurable: true
    })
  }

  it('opens the panel with a shell already in it', async () => {
    const createShellTerminal = vi
      .fn()
      .mockResolvedValue({ id: 'sh1', agentType: 'shell', projectName: 'p', projectPath: '/proj' })
    seedApi(createShellTerminal)
    seedProject(WEB)
    act(() => {
      useAppStore.setState({ terminalsPanes: new Map() })
    })

    render(<SessionItem session={session} />)
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Show terminals for/ }))
    })

    // An empty panel is a box occupying a pane and showing nothing, so opening
    // it and creating the first shell are one action.
    expect(createShellTerminal).toHaveBeenCalledWith('/proj')
    expect(useAppStore.getState().terminalsPanes.get(session.id)?.terminals).toEqual(['sh1'])
  })

  it('offers a shell neither a panel nor a browser', () => {
    seedApi(vi.fn())
    seedProject(WEB)
    act(() => {
      useAppStore.setState({ terminalsPanes: new Map(), browserPanes: new Map() })
    })

    render(<SessionItem session={{ ...session, agentType: 'shell' }} />)

    // A panel of shells inside a shell, and a browser beside a prompt nobody
    // asked to drive from there. The file tree stays — looking at files next to
    // a shell is as reasonable as next to an agent.
    expect(screen.queryByRole('button', { name: /terminals for/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /browser for/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /files for/ })).toBeInTheDocument()
  })

  it('leaves a shell the control for a panel it already has', () => {
    seedApi(vi.fn())
    seedProject(WEB)
    act(() => {
      useAppStore.setState({ terminalsPanes: new Map() })
      useAppStore.getState().openTerminalsPane(session.id, 'sh1')
    })

    render(<SessionItem session={{ ...session, agentType: 'shell' }} />)

    // Hiding it would leave the panel on screen with no way to close it.
    expect(screen.getByRole('button', { name: /Close terminals for/ })).toBeInTheDocument()
  })

  it('closes an open panel and the shells in it, rather than adding another', async () => {
    const createShellTerminal = vi.fn()
    const killTerminal = vi.fn().mockResolvedValue(undefined)
    seedApi(createShellTerminal)
    Object.assign((window as unknown as { api: Record<string, unknown> }).api, { killTerminal })
    seedProject(WEB)
    act(() => {
      useAppStore.setState({ terminalsPanes: new Map() })
      useAppStore.getState().openTerminalsPane(session.id, 'sh1')
    })

    render(<SessionItem session={session} />)
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Close terminals for/ }))
    })

    // Dropping the claim alone would scatter live shells across the grid as
    // top-level cards, and the next click here would add another beside them.
    expect(useAppStore.getState().terminalsPanes.has(session.id)).toBe(false)
    expect(killTerminal).toHaveBeenCalledWith('sh1')
    expect(createShellTerminal).not.toHaveBeenCalled()
  })
})

/**
 * A session's popped-out cards are listed beneath its row, so the row is now a
 * fragment rather than a single button — and the rows below it have to be
 * reachable, not merely rendered.
 */
describe('SessionItem with popped-out cards', () => {
  beforeEach(() => {
    seedProject(WEB)
    // `seedProject` resets the session maps but not the pane ones, so a card
    // promoted in one test would otherwise still be listed in the next.
    act(() => useAppStore.setState({ editorPanes: new Map(), browserPanes: new Map() }))
  })

  it('lists a card beneath the session it came from', () => {
    act(() => {
      useAppStore.getState().promoteFile(session.id, '/proj/src/server.ts')
    })
    render(<SessionItem session={session} />)

    expect(screen.getByText('My Session')).toBeInTheDocument()
    expect(screen.getByText('server.ts')).toBeInTheDocument()
  })

  it('lists a popped-out page by its host', () => {
    act(() => {
      useAppStore.getState().openBrowserPane(session.id, 'vorn.dev')
      useAppStore.getState().promoteBrowserTab(session.id, 0)
    })
    render(<SessionItem session={session} />)

    expect(screen.getByText('vorn.dev')).toBeInTheDocument()
  })

  it("does not list another session's cards", () => {
    act(() => {
      useAppStore.setState({
        terminals: new Map([
          ...useAppStore.getState().terminals,
          [
            'other',
            {
              id: 'other',
              session: { id: 'other', projectPath: '/p2', projectName: 'p2', agentType: 'claude' },
              status: 'idle',
              lastOutputTimestamp: 1
            }
          ]
        ]) as never
      })
      useAppStore.getState().promoteFile('other', '/p2/theirs.ts')
    })
    render(<SessionItem session={session} />)

    expect(screen.queryByText('theirs.ts')).not.toBeInTheDocument()
  })

  it('leaves the row alone when the session has popped nothing out', () => {
    render(<SessionItem session={session} />)
    // The session row itself, and nothing beneath it. (Its own controls are
    // real buttons; a card row is a div with role=button, which is the tell.)
    expect(screen.getByText('My Session')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /back in its session card/ })).toBeNull()
  })
})

/**
 * The row's own controls. These predate cards, but the row became a fragment to
 * carry them, so they are worth pinning while the file is being changed.
 */
describe('SessionItem controls', () => {
  beforeEach(() => {
    seedProject(WEB)
    act(() =>
      useAppStore.setState({
        editorPanes: new Map(),
        browserPanes: new Map(),
        filesPanes: new Set(),
        focusedTerminalId: null,
        activeTabId: null,
        config: { defaults: { layoutMode: 'grid' } } as never
      })
    )
  })

  it('focuses the session in grid mode', () => {
    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByText('My Session'))
    expect(useAppStore.getState().focusedTerminalId).toBe(session.id)
  })

  it('activates its tab in tab mode, and leaves focus alone', () => {
    act(() => useAppStore.setState({ config: { defaults: { layoutMode: 'tabs' } } as never }))
    render(<SessionItem session={session} />)
    fireEvent.click(screen.getByText('My Session'))

    expect(useAppStore.getState().activeTabId).toBe(session.id)
    expect(useAppStore.getState().focusedTerminalId).toBeNull()
  })

  it('toggles the file tree from the row', () => {
    render(<SessionItem session={session} />)
    const files = screen.getByRole('button', { name: /files for My Session/ })

    fireEvent.click(files)
    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: /files for My Session/ }))
    expect(useAppStore.getState().filesPanes.has(session.id)).toBe(false)
  })

  it('toggles the browser from the row', () => {
    render(<SessionItem session={session} />)
    const browser = screen.getByRole('button', { name: /browser for My Session/ })

    fireEvent.click(browser)
    expect(useAppStore.getState().browserPanes.has(session.id)).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: /browser for My Session/ }))
    expect(useAppStore.getState().browserPanes.has(session.id)).toBe(false)
  })
})
