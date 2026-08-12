// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act, waitFor } from '@testing-library/react'
import type { FileEntry } from '../src/shared/types'

// AgentCard (pulled in by focus mode) reads matchMedia at module load, so it
// must exist before the dynamic imports below.
Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})

const mockListDir = vi.fn<(path: string) => Promise<FileEntry[]>>()
const mockReadFileContent = vi.fn<(path: string) => Promise<string | null>>()
const mockStartPick = vi.fn<(sessionId: string) => Promise<unknown>>()
const mockWriteTerminal = vi.fn()
const mockAnnotate = vi.fn<(p: unknown) => Promise<unknown>>()

Object.defineProperty(window, 'api', {
  value: {
    listDir: (...args: unknown[]) => mockListDir(...(args as [string])),
    readFileContent: (...args: unknown[]) => mockReadFileContent(...(args as [string])),
    writeFileContent: vi.fn().mockResolvedValue({ success: true }),
    notifyWidgetStatus: vi.fn(),
    // The browser pane reports its guest to main so the agent can drive it.
    attachBrowser: vi.fn(),
    detachBrowser: vi.fn(),
    startBrowserPick: (...args: unknown[]) => mockStartPick(...(args as [string])),
    cancelBrowserPick: vi.fn(),
    annotateBrowser: (...args: unknown[]) => mockAnnotate(args[0]),
    writeTerminal: (...args: unknown[]) => mockWriteTerminal(...args),
    // FocusedTerminal pulls in the full card header, which probes the host.
    detectInstalledAgents: vi.fn().mockResolvedValue([]),
    detectIDEs: vi.fn().mockResolvedValue([]),
    getGitDiffStat: vi.fn().mockResolvedValue(null),
    onTerminalData: vi.fn(() => () => {}),
    onTerminalExit: vi.fn(() => () => {})
  },
  writable: true,
  configurable: true
})

const { useAppStore } = await import('../src/renderer/stores')
const { activeBrowserUrl } = await import('../src/renderer/stores/types')

/** The page a session's browser is showing, i.e. its active tab. */
const browserUrl = (id: string): string | null =>
  activeBrowserUrl(useAppStore.getState().browserPanes.get(id))
const { FilesCard } = await import('../src/renderer/components/FilesCard')
const { EditorCard } = await import('../src/renderer/components/EditorCard')
const { BrowserCard } = await import('../src/renderer/components/BrowserCard')
const { FocusedTerminal } = await import('../src/renderer/components/FocusedTerminal')
const { dirtyRefFor, clearDirty } = await import('../src/renderer/lib/editor-dirty')

const ENTRIES: FileEntry[] = [
  { name: 'src', path: '/repo/src', isDirectory: true },
  { name: 'a.ts', path: '/repo/a.ts', isDirectory: false },
  { name: 'b.ts', path: '/repo/b.ts', isDirectory: false }
]

const session = (id: string, over: Record<string, unknown> = {}) =>
  ({
    id,
    projectName: 'repo',
    projectPath: '/repo',
    agentType: 'claude',
    createdAt: 0,
    displayName: id,
    ...over
  }) as never

function seed(ids = ['t1']): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, { id, session: session(id), status: 'idle', lastOutputTimestamp: 1 })
  }
  act(() => {
    useAppStore.setState({
      terminals,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      minimizedTerminals: new Set(),
      maximizedPaneId: null,
      terminalOrder: ids
    })
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  clearDirty('t1')
  mockListDir.mockResolvedValue(ENTRIES)
  mockReadFileContent.mockResolvedValue('hello')
  seed()
})

describe('FilesCard', () => {
  it('lists the owner session’s worktree and opens a clicked file in its editor', async () => {
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    // cwd comes from the owner session, not a global — this is what lets two
    // sessions show different trees.
    expect(mockListDir).toHaveBeenCalledWith('/repo', undefined)

    fireEvent.click(screen.getByText('a.ts'))
    expect(useAppStore.getState().editorPanes.get('t1')?.filePath).toBe('/repo/a.ts')
  })

  it('prefers the worktree path when the session has one', async () => {
    const terminals = new Map()
    terminals.set('t1', {
      id: 't1',
      session: session('t1', { worktreePath: '/repo-wt' }),
      status: 'idle',
      lastOutputTimestamp: 1
    })
    act(() => useAppStore.setState({ terminals }))

    render(<FilesCard sessionId="t1" />)
    await waitFor(() => expect(mockListDir).toHaveBeenCalledWith('/repo-wt', undefined))
  })

  it('confirms before replacing a dirty editor, and keeps the old file on cancel', async () => {
    act(() => useAppStore.getState().openEditorPane('t1', '/repo/a.ts'))
    dirtyRefFor('t1').current = true
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)

    render(<FilesCard sessionId="t1" />)
    await screen.findByText('b.ts')
    fireEvent.click(screen.getByText('b.ts'))

    expect(confirm).toHaveBeenCalled()
    // Cancelling must not throw the unsaved buffer away.
    expect(useAppStore.getState().editorPanes.get('t1')?.filePath).toBe('/repo/a.ts')
  })

  it('closes its own pane without touching the editor', async () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/repo/a.ts')
    })
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    fireEvent.click(screen.getByRole('button', { name: /Close Files/ }))
    expect(useAppStore.getState().filesPanes.has('t1')).toBe(false)
    // The panes are independent — the open file survives.
    expect(useAppStore.getState().editorPanes.has('t1')).toBe(true)
  })

  it('renders nothing when its owner session is gone', () => {
    const { container } = render(<FilesCard sessionId="ghost" />)
    expect(container).toBeEmptyDOMElement()
  })
})

describe('EditorCard', () => {
  beforeEach(() => {
    act(() => useAppStore.getState().openEditorPane('t1', '/repo/a.ts'))
  })

  it('titles the pane with the filename and loads its content', async () => {
    render(<EditorCard sessionId="t1" />)
    await waitFor(() =>
      expect(mockReadFileContent).toHaveBeenCalledWith('/repo/a.ts', undefined, undefined)
    )
    // The filename lives in the header; the path strip below shows the rest, so
    // the two must not duplicate it.
    expect(screen.getAllByText('a.ts').length).toBeGreaterThan(0)
  })

  it('confirms before closing a dirty buffer and stays open on cancel', async () => {
    render(<EditorCard sessionId="t1" />)
    await waitFor(() => expect(mockReadFileContent).toHaveBeenCalled())
    dirtyRefFor('t1').current = true
    vi.spyOn(window, 'confirm').mockReturnValue(false)

    fireEvent.click(screen.getByRole('button', { name: /Close a\.ts/ }))
    expect(useAppStore.getState().editorPanes.has('t1')).toBe(true)
  })

  it('closes when the discard is confirmed', async () => {
    render(<EditorCard sessionId="t1" />)
    await waitFor(() => expect(mockReadFileContent).toHaveBeenCalled())
    dirtyRefFor('t1').current = true
    vi.spyOn(window, 'confirm').mockReturnValue(true)

    fireEvent.click(screen.getByRole('button', { name: /Close a\.ts/ }))
    expect(useAppStore.getState().editorPanes.has('t1')).toBe(false)
  })

  it('renders nothing when no file is open', () => {
    act(() => useAppStore.getState().closeEditorPane('t1'))
    const { container } = render(<EditorCard sessionId="t1" />)
    expect(container).toBeEmptyDOMElement()
  })
})

describe('PaneCard chrome', () => {
  it('maximizes and restores through the store', async () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    fireEvent.click(screen.getByRole('button', { name: /Maximize Files/ }))
    expect(useAppStore.getState().maximizedPaneId).toBe('files:t1')

    fireEvent.click(screen.getByRole('button', { name: /Restore Files/ }))
    expect(useAppStore.getState().maximizedPaneId).toBeNull()
  })

  it('seats the tree pane controls in its filter row, not a second bar', async () => {
    // The filter row is already a full-width bar. A title row above it is
    // chrome stacked on chrome, and costs a line of tree.
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    const header = screen.getByTestId('files-pane-header')
    expect(header).toContainElement(screen.getByLabelText('Maximize Files'))
    expect(header).toContainElement(screen.getByLabelText('Close Files'))
    expect(screen.getByPlaceholderText('Filter files…')).toBeInTheDocument()
  })

  it('names the open file once, in the path strip that carries its controls', async () => {
    // The strip already shows the path, icon and dirty dot; a header above it
    // repeated the filename directly over itself.
    act(() => useAppStore.getState().openEditorPane('t1', '/repo/a.ts'))
    render(<EditorCard sessionId="t1" />)

    const header = await screen.findByTestId('editor-pane-header')
    expect(header).toContainElement(screen.getByLabelText('Maximize a.ts'))
    expect(screen.getAllByText('a.ts')).toHaveLength(1)
  })

  it('offers no minimize, because a minimized pane had nowhere to go', async () => {
    // The dock only surfaces sessions and expanded mode ignores the minimized
    // set entirely, so the button silently discarded the pane.
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    expect(screen.queryByRole('button', { name: /Minimize Files/ })).toBeNull()
  })
})

describe('PaneCard drag and double-click', () => {
  it("reports drags with its own pane id, not its owner's", async () => {
    const onDragStart = vi.fn()
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" onDragStart={onDragStart} />)
    await screen.findByText('a.ts')

    fireEvent.pointerDown(screen.getByTestId('files-pane-header'))
    expect(onDragStart).toHaveBeenCalledWith('files:t1', expect.anything())
  })

  it('toggles maximize on header double-click', async () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    fireEvent.doubleClick(screen.getByTestId('files-pane-header'))
    expect(useAppStore.getState().maximizedPaneId).toBe('files:t1')

    fireEvent.doubleClick(screen.getByTestId('files-pane-header'))
    expect(useAppStore.getState().maximizedPaneId).toBeNull()
  })
})

describe('BrowseFilesButton', () => {
  it("toggles the session's files pane", async () => {
    const { BrowseFilesButton } = await import('../src/renderer/components/GitChangesIndicator')
    render(<BrowseFilesButton terminalId="t1" />)

    fireEvent.click(screen.getByRole('button', { name: 'Browse files' }))
    expect(useAppStore.getState().filesPanes.has('t1')).toBe(true)

    fireEvent.click(screen.getByRole('button', { name: 'Browse files' }))
    expect(useAppStore.getState().filesPanes.has('t1')).toBe(false)
  })
})

describe('panes travel with their session into focus mode', () => {
  beforeEach(() => {
    act(() =>
      useAppStore.setState({
        focusedTerminalId: 't1',
        previewTerminalId: null,
        config: { defaults: { domBlockRendering: false } } as never
      })
    )
  })

  it("renders the session's tree and file beside the expanded terminal", async () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/repo/a.ts')
    })

    render(<FocusedTerminal />)

    // Expanding a card used to hide whatever the session had open next to it.
    expect(await screen.findByTestId('files-pane-header')).toBeInTheDocument()
    await waitFor(() =>
      expect(mockReadFileContent).toHaveBeenCalledWith('/repo/a.ts', undefined, undefined)
    )
  })

  it("carries the session's browser into focus mode too", () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<FocusedTerminal />)
    expect(screen.getByLabelText('Address')).toHaveValue('http://localhost:5173/')
  })

  it('gives a maximized pane the whole stage and hides its siblings', async () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1', 'localhost:5173')
      useAppStore.getState().setMaximizedPane('browser:t1')
    })

    render(<FocusedTerminal />)

    // Focus mode used to render panes in a fixed rail and ignore the maximize
    // entirely, so the button looked dead once a card was expanded.
    expect(screen.getByLabelText('Address')).toBeInTheDocument()
    // Hidden, but still mounted: unmounting a pane to hide it costs the browser
    // its live guest and the agent its CDP handle. Visually gone is enough.
    const files = await screen.findByTestId('files-pane-header')
    expect(files.closest('[aria-hidden]')).not.toBeNull()
    // The terminal gives up its space too — a maximized pane covering only the
    // rail is not a maximize.
    expect(screen.getByTestId('focused-terminal-column')).toHaveClass('hidden')
  })

  it('keeps the browser guest alive when a sibling pane is maximized', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openBrowserPane('t1', 'localhost:5173')
      useAppStore.getState().setMaximizedPane('files:t1')
    })

    render(<FocusedTerminal />)

    // The <webview> must survive. Unmounting it destroys the guest, which loses
    // the page and scroll position for the person and detaches CDP for the
    // agent — which is then told "no pane open" while the store still says one
    // exists, with no way to recover until the person un-maximizes.
    expect(document.querySelectorAll('webview')).toHaveLength(1)
    expect(screen.getByLabelText('Address').closest('[aria-hidden]')).not.toBeNull()
  })

  it("ignores another session's maximized pane", async () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      // t1 owns a browser too, so only the *owner* check can keep this stage
      // whole — matching on kind alone would blank it.
      useAppStore.getState().openBrowserPane('t1', 'localhost:5173')
      useAppStore.getState().setMaximizedPane('browser:t2')
    })

    render(<FocusedTerminal />)
    // A stale or foreign id must not blank the stage of the session in focus.
    expect(await screen.findByTestId('files-pane-header')).toBeInTheDocument()
  })

  it('shows no pane column when the session has none open', () => {
    render(<FocusedTerminal />)
    expect(screen.queryByTestId('files-pane-header')).not.toBeInTheDocument()
  })

  it('renders nothing when no session is focused', () => {
    act(() => useAppStore.setState({ focusedTerminalId: null }))
    const { container } = render(<FocusedTerminal />)
    expect(container).toBeEmptyDOMElement()
  })
})

describe('BrowserCard', () => {
  it('renders the page host on its tab and the url in the address bar', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    // The tab strip is this pane's only title bar — a second header above it
    // would be chrome stacked on chrome, so the host is named exactly once.
    expect(screen.getAllByText('localhost:5173')).toHaveLength(1)
    expect(screen.getByRole('tab')).toHaveTextContent('localhost:5173')
    expect(screen.getByLabelText('Address')).toHaveValue('http://localhost:5173/')
  })

  it("sends what the user picked to the session's agent", async () => {
    mockStartPick.mockResolvedValueOnce({
      url: 'http://localhost:5173/',
      rect: { x: 0, y: 0, width: 80, height: 24 },
      text: 'Save',
      selector: 'form > button.primary',
      outerHTML: '<button class="primary">Save</button>',
      tagName: 'button',
      componentName: 'SaveButton',
      source: 'src/Save.tsx:12'
    })
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('Pick an element for the agent'))

    // It goes in as a message, not as a command: the person is saying "this
    // one", and what to do about it is still the agent's call.
    await waitFor(() => expect(mockWriteTerminal).toHaveBeenCalled())
    const sent = mockWriteTerminal.mock.calls[0][1] as string
    expect(sent).toContain('form > button.primary')
    expect(sent).toContain('SaveButton')
    expect(sent).toContain('src/Save.tsx:12')
  })

  it('says nothing to the agent when the pick is cancelled', async () => {
    mockStartPick.mockResolvedValueOnce(null)
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('Pick an element for the agent'))

    // Escaping out of a picker is an ordinary thing to do; interrupting the
    // agent with an empty selection would punish it.
    await waitFor(() => expect(mockStartPick).toHaveBeenCalledWith('t1'))
    expect(mockWriteTerminal).not.toHaveBeenCalled()
  })

  it('sends the ink and what it covers to the agent', async () => {
    // jsdom has no 2d context; the drawing is cosmetic, the stroke record is not.
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({
      clearRect: vi.fn(),
      beginPath: vi.fn(),
      moveTo: vi.fn(),
      lineTo: vi.fn(),
      stroke: vi.fn()
    })) as unknown as HTMLCanvasElement['getContext']
    mockAnnotate.mockResolvedValueOnce({
      url: 'http://localhost:5173/',
      elements: [{ role: 'button', name: 'Save' }],
      image: '',
      bounds: { x: 0, y: 0, width: 10, height: 10 }
    })
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('Draw on the page for the agent'))
    const ink = screen.getByTestId('browser-ink')
    ink.setPointerCapture = vi.fn()
    fireEvent.pointerDown(ink, { clientX: 10, clientY: 10 })
    fireEvent.pointerMove(ink, { clientX: 40, clientY: 40 })
    fireEvent.pointerUp(ink)
    fireEvent.click(screen.getByLabelText('Send the annotation'))

    await waitFor(() => expect(mockWriteTerminal).toHaveBeenCalled())
    expect((mockAnnotate.mock.calls[0][0] as { strokes: unknown[] }).strokes).toHaveLength(1)
    expect(mockWriteTerminal.mock.calls[0][1] as string).toContain('Save')
  })

  it('does not interrupt the agent when the pencil is armed and nothing is drawn', async () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    // Arming and thinking better of it is an ordinary thing to do.
    fireEvent.click(screen.getByLabelText('Draw on the page for the agent'))
    fireEvent.click(screen.getByLabelText('Send the annotation'))

    expect(mockAnnotate).not.toHaveBeenCalled()
    expect(mockWriteTerminal).not.toHaveBeenCalled()
    expect(screen.queryByTestId('browser-ink')).toBeNull()
  })

  it('calls an unvisited tab "New tab" and leaves its address bar empty', () => {
    act(() => useAppStore.getState().openBrowserPane('t1'))
    render(<BrowserCard sessionId="t1" />)

    // "about:blank" is jargon, and pre-filling it gives the user a string to
    // delete before they can type the address they actually want.
    expect(screen.getByRole('tab')).toHaveTextContent('New tab')
    expect(screen.getByLabelText('Address')).toHaveValue('')
  })

  it('seats the pane controls in the tab strip rather than a second header', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    // Maximize / close still have to be reachable once the pane's own header
    // is gone.
    expect(screen.getByLabelText('Maximize localhost:5173')).toBeInTheDocument()
    expect(screen.getByLabelText('Close localhost:5173')).toBeInTheDocument()
  })

  it('opens a second tab and switches between them without losing either page', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('New tab'))
    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))

    const tabs = screen.getAllByRole('tab')
    expect(tabs).toHaveLength(2)
    expect(tabs[1]).toHaveAttribute('aria-selected', 'true')

    fireEvent.click(tabs[0])
    // Switching tabs must put the address bar on the page you switched to.
    expect(screen.getByLabelText('Address')).toHaveValue('http://localhost:5173/')

    // Both pages stay mounted so a switch back doesn't reload them.
    expect(document.querySelectorAll('webview')).toHaveLength(2)
  })

  it('closes a tab from its own button', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    act(() => useAppStore.getState().addBrowserTab('t1', 'example.com'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('Close tab example.com'))
    expect(useAppStore.getState().browserPanes.get('t1')?.tabs).toEqual(['http://localhost:5173/'])
  })

  it('navigates on submit, normalizing what was typed', () => {
    act(() => useAppStore.getState().openBrowserPane('t1'))
    render(<BrowserCard sessionId="t1" />)

    const input = screen.getByLabelText('Address')
    fireEvent.change(input, { target: { value: 'example.com/docs' } })
    fireEvent.submit(input)

    expect(browserUrl('t1')).toBe('https://example.com/docs')
  })

  it('explains a rejected address instead of silently doing nothing', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))
    render(<BrowserCard sessionId="t1" />)

    const input = screen.getByLabelText('Address')
    fireEvent.change(input, { target: { value: 'file:///etc/passwd' } })
    fireEvent.submit(input)

    expect(screen.getByText(/does not look like a web address/)).toBeInTheDocument()
    // The pane keeps the page it had.
    expect(browserUrl('t1')).toBe('https://example.com/')
  })

  it('closes its own pane', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByRole('button', { name: /Close example\.com/ }))
    expect(useAppStore.getState().browserPanes.has('t1')).toBe(false)
  })

  it('starts with navigation disabled until the guest reports history', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))
    render(<BrowserCard sessionId="t1" />)

    expect(screen.getByRole('button', { name: 'Go back' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Go forward' })).toBeDisabled()
  })

  it('renders nothing when the session has no browser open', () => {
    const { container } = render(<BrowserCard sessionId="t1" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('renders nothing when its owner session is gone', () => {
    act(() => useAppStore.getState().openBrowserPane('ghost', 'example.com'))
    const { container } = render(<BrowserCard sessionId="ghost" />)
    expect(container).toBeEmptyDOMElement()
  })
})
