// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act, waitFor } from '@testing-library/react'
import type { FileEntry } from '../src/shared/types'

// AgentCard (pulled in by PaneRenderer) reads matchMedia at module load, so it
// must exist before the dynamic imports below.
Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})

const mockListDir = vi.fn<(path: string) => Promise<FileEntry[]>>()
const mockReadFileContent = vi.fn<(path: string) => Promise<string | null>>()

Object.defineProperty(window, 'api', {
  value: {
    listDir: (...args: unknown[]) => mockListDir(...(args as [string])),
    readFileContent: (...args: unknown[]) => mockReadFileContent(...(args as [string])),
    writeFileContent: vi.fn().mockResolvedValue({ success: true }),
    notifyWidgetStatus: vi.fn(),
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
const { FilesCard } = await import('../src/renderer/components/FilesCard')
const { EditorCard } = await import('../src/renderer/components/EditorCard')
const { PaneRenderer } = await import('../src/renderer/components/PaneRenderer')
const { BrowserCard } = await import('../src/renderer/components/BrowserCard')
const { MinimizedPill } = await import('../src/renderer/components/MinimizedPill')
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
  it('maximizes, restores and minimizes through the store', async () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    fireEvent.click(screen.getByRole('button', { name: /Maximize Files/ }))
    expect(useAppStore.getState().maximizedPaneId).toBe('files:t1')

    fireEvent.click(screen.getByRole('button', { name: /Restore Files/ }))
    expect(useAppStore.getState().maximizedPaneId).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: /Minimize Files/ }))
    expect(useAppStore.getState().minimizedTerminals.has('files:t1')).toBe(true)
  })
})

describe('PaneCard drag and double-click', () => {
  it("reports drags with its own pane id, not its owner's", async () => {
    const onDragStart = vi.fn()
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" onDragStart={onDragStart} />)
    await screen.findByText('a.ts')

    fireEvent.pointerDown(screen.getByText('Files'))
    expect(onDragStart).toHaveBeenCalledWith('files:t1', expect.anything())
  })

  it('toggles maximize on header double-click', async () => {
    act(() => useAppStore.getState().openFilesPane('t1'))
    render(<FilesCard sessionId="t1" />)
    await screen.findByText('a.ts')

    fireEvent.doubleClick(screen.getByText('Files'))
    expect(useAppStore.getState().maximizedPaneId).toBe('files:t1')

    fireEvent.doubleClick(screen.getByText('Files'))
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

describe('PaneRenderer', () => {
  it('dispatches on pane kind', async () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().openEditorPane('t1', '/repo/a.ts')
    })

    const files = render(<PaneRenderer paneId="files:t1" />)
    expect(await files.findByText('Files')).toBeInTheDocument()
    files.unmount()

    const editor = render(<PaneRenderer paneId="editor:t1" />)
    await waitFor(() => expect(mockReadFileContent).toHaveBeenCalled())
    editor.unmount()

    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))
    const browser = render(<PaneRenderer paneId="browser:t1" />)
    expect(browser.getByLabelText('Address')).toBeInTheDocument()
    browser.unmount()
  })
})

describe('MinimizedPill for child panes', () => {
  it('labels a minimized tree with its owner and restores on click', () => {
    act(() => {
      useAppStore.getState().openFilesPane('t1')
      useAppStore.getState().toggleMinimized('files:t1')
    })

    render(<MinimizedPill terminalId="files:t1" />)
    // Without the kind branch this pill renders nothing and the pane is lost.
    expect(screen.getByText('Files')).toBeInTheDocument()
    expect(screen.getByText('t1')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button'))
    expect(useAppStore.getState().minimizedTerminals.has('files:t1')).toBe(false)
  })

  it('labels a minimized editor with its filename', () => {
    act(() => {
      useAppStore.getState().openEditorPane('t1', '/repo/a.ts')
      useAppStore.getState().toggleMinimized('editor:t1')
    })

    render(<MinimizedPill terminalId="editor:t1" />)
    expect(screen.getByText('a.ts')).toBeInTheDocument()
  })

  it('renders nothing for a pane whose session is gone', () => {
    const { container } = render(<MinimizedPill terminalId="files:ghost" />)
    expect(container).toBeEmptyDOMElement()
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
    expect(await screen.findByText('Files')).toBeInTheDocument()
    await waitFor(() =>
      expect(mockReadFileContent).toHaveBeenCalledWith('/repo/a.ts', undefined, undefined)
    )
  })

  it("carries the session's browser into focus mode too", () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<FocusedTerminal />)
    expect(screen.getByLabelText('Address')).toHaveValue('http://localhost:5173/')
  })

  it('shows no pane column when the session has none open', () => {
    render(<FocusedTerminal />)
    expect(screen.queryByText('Files')).not.toBeInTheDocument()
  })

  it('renders nothing when no session is focused', () => {
    act(() => useAppStore.setState({ focusedTerminalId: null }))
    const { container } = render(<FocusedTerminal />)
    expect(container).toBeEmptyDOMElement()
  })
})

describe('BrowserCard', () => {
  it('renders the page host in its header and the url in the address bar', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'localhost:5173'))
    render(<BrowserCard sessionId="t1" />)

    // Headers are narrow, so the host is what earns the space.
    expect(screen.getByText('localhost:5173')).toBeInTheDocument()
    expect(screen.getByLabelText('Address')).toHaveValue('http://localhost:5173/')
  })

  it('navigates on submit, normalizing what was typed', () => {
    act(() => useAppStore.getState().openBrowserPane('t1'))
    render(<BrowserCard sessionId="t1" />)

    const input = screen.getByLabelText('Address')
    fireEvent.change(input, { target: { value: 'example.com/docs' } })
    fireEvent.submit(input)

    expect(useAppStore.getState().browserPanes.get('t1')?.url).toBe('https://example.com/docs')
  })

  it('explains a rejected address instead of silently doing nothing', () => {
    act(() => useAppStore.getState().openBrowserPane('t1', 'example.com'))
    render(<BrowserCard sessionId="t1" />)

    const input = screen.getByLabelText('Address')
    fireEvent.change(input, { target: { value: 'file:///etc/passwd' } })
    fireEvent.submit(input)

    expect(screen.getByText(/does not look like a web address/)).toBeInTheDocument()
    // The pane keeps the page it had.
    expect(useAppStore.getState().browserPanes.get('t1')?.url).toBe('https://example.com/')
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

describe('MinimizedPill fallback labels', () => {
  it('names a browser with no page yet, and an editor with no file', () => {
    act(() => {
      useAppStore.setState({
        browserPanes: new Map([['t1', { url: '' }]]),
        editorPanes: new Map([['t1', { filePath: '' }]]),
        minimizedTerminals: new Set(['browser:t1', 'editor:t1'])
      })
    })

    const browser = render(<MinimizedPill terminalId="browser:t1" />)
    expect(browser.getByText('Browser')).toBeInTheDocument()
    browser.unmount()

    const editor = render(<MinimizedPill terminalId="editor:t1" />)
    expect(editor.getByText('File')).toBeInTheDocument()
  })
})

describe('MinimizedPill for a browser pane', () => {
  it('labels the pill with the page host', () => {
    act(() => {
      useAppStore.getState().openBrowserPane('t1', 'localhost:5173')
      useAppStore.getState().toggleMinimized('browser:t1')
    })

    render(<MinimizedPill terminalId="browser:t1" />)
    expect(screen.getByText('localhost:5173')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button'))
    expect(useAppStore.getState().minimizedTerminals.has('browser:t1')).toBe(false)
  })
})
