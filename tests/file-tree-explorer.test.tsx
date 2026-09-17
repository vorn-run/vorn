// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { FileEntry } from '../src/shared/types'

// Replace Node's experimental localStorage (which needs a file path)
// with an in-memory shim so the component's getItem/setItem work in tests.
{
  const store = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => Array.from(store.keys())[i] ?? null,
      get length() {
        return store.size
      }
    }
  })
}

// Stub shiki so highlightCode resolves without WASM in jsdom.
vi.mock('shiki', () => ({
  createHighlighter: async () => ({
    loadLanguage: async () => undefined,
    codeToTokens: () => ({ tokens: [] })
  }),
  createJavaScriptRegexEngine: () => ({})
}))

// jsdom doesn't implement scrollIntoView; the find-cycle effect calls it.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function () {}
}

const mockListDir = vi.fn<(path: string) => Promise<FileEntry[]>>()
const mockReadFileContent = vi.fn<(path: string) => Promise<string | null>>()
const mockWriteFileContent =
  vi.fn<(path: string, content: string) => Promise<{ success: boolean; error?: string }>>()
const mockFileStamp = vi.fn<(path: string) => Promise<{ size: number; mtimeMs: number } | null>>()

Object.defineProperty(window, 'api', {
  value: {
    listDir: (...args: unknown[]) => mockListDir(...(args as [string])),
    readFileContent: (...args: unknown[]) => mockReadFileContent(...(args as [string])),
    writeFileContent: (...args: unknown[]) => mockWriteFileContent(...(args as [string, string])),
    fileStamp: (...args: unknown[]) => mockFileStamp(...(args as [string])),
    notifyWidgetStatus: vi.fn()
  },
  writable: true,
  configurable: true
})

const { useAppStore } = await import('../src/renderer/stores')
const { FilesCard } = await import('../src/renderer/components/FilesCard')
const { truncationMarker } = await import('../packages/shared/src/string-utils')

const ROOT_ENTRIES: FileEntry[] = [
  { name: 'src', path: '/repo/src', isDirectory: true },
  { name: 'tests', path: '/repo/tests', isDirectory: true },
  { name: 'README.md', path: '/repo/README.md', isDirectory: false }
]

const SRC_CHILDREN: FileEntry[] = [
  { name: 'index.ts', path: '/repo/src/index.ts', isDirectory: false },
  { name: 'utils.ts', path: '/repo/src/utils.ts', isDirectory: false }
]

const CONTENT = 'hello\nworld\nworld again'

beforeEach(() => {
  mockListDir.mockReset()
  mockReadFileContent.mockReset()
  mockWriteFileContent.mockReset()
  mockFileStamp.mockReset()
  localStorage.clear()

  mockListDir.mockImplementation(async (path: string) => {
    if (path === '/repo') return ROOT_ENTRIES
    if (path === '/repo/src') return SRC_CHILDREN
    return []
  })
  mockReadFileContent.mockResolvedValue(CONTENT)
  mockWriteFileContent.mockResolvedValue({ success: true })
  mockFileStamp.mockResolvedValue({ size: 23, mtimeMs: 1 })

  const terminals = new Map()
  terminals.set('t1', {
    id: 't1',
    session: { id: 't1', projectName: 'repo', projectPath: '/repo', agentType: 'claude' },
    status: 'idle',
    lastOutputTimestamp: 1
  })
  act(() => {
    useAppStore.setState({
      terminals: terminals as never,
      filesPanes: new Map(),
      editorPanes: new Map(),
      maximizedPaneId: null,
      config: { defaults: {} } as never
    })
    useAppStore.getState().openFilesPane('t1')
  })
})

afterEach(() => {
  vi.restoreAllMocks()
})

async function renderPane(): Promise<void> {
  await act(async () => {
    render(<FilesCard sessionId="t1" />)
  })
  await screen.findByText('README.md')
}

async function openReadme(): Promise<HTMLTextAreaElement> {
  await act(async () => {
    fireEvent.click(screen.getByText('README.md'))
  })
  const editor = (await screen.findByLabelText('Edit README.md')) as HTMLTextAreaElement
  await waitFor(() => expect(editor.value).toBe(CONTENT))
  return editor
}

describe('the Files pane', () => {
  it('renders root entries with chevron-only directories', async () => {
    await renderPane()
    expect(screen.getByText('src')).toBeInTheDocument()
    expect(screen.getByText('tests')).toBeInTheDocument()
    expect(screen.getByText('README.md')).toBeInTheDocument()
  })

  it('filters tree by name and shows empty hint when no matches', async () => {
    await renderPane()
    const filter = screen.getByPlaceholderText('Filter files…')

    await act(async () => {
      fireEvent.change(filter, { target: { value: 'README' } })
    })
    expect(screen.getByText('README.md')).toBeInTheDocument()
    expect(screen.queryByText('src')).not.toBeInTheDocument()

    await act(async () => {
      fireEvent.change(filter, { target: { value: 'zzznomatch' } })
    })
    expect(screen.getByText(/No matching files loaded/)).toBeInTheDocument()
  })

  it('expands a directory on chevron click and lists children', async () => {
    await renderPane()
    await act(async () => {
      fireEvent.click(screen.getByText('src'))
    })
    await screen.findByText('index.ts')
    expect(screen.getByText('utils.ts')).toBeInTheDocument()
    expect(mockListDir).toHaveBeenCalledWith('/repo/src', undefined)
  })

  it('opens a file ready to type in, beside the tree it came from', async () => {
    await renderPane()
    expect(screen.queryByRole('textbox', { name: /^Edit / })).not.toBeInTheDocument()

    await openReadme()
    // The path strip names the file relative to the worktree.
    expect(screen.getByRole('tabpanel', { name: 'README.md' })).toHaveTextContent('README.md')
    expect(mockReadFileContent).toHaveBeenCalledWith('/repo/README.md', undefined, undefined)
    expect(screen.queryByLabelText('Edit')).not.toBeInTheDocument()
    expect(screen.getByTestId('files-tree-column')).not.toHaveClass('hidden')
  })

  it('draws what is typed, so the text under the caret is never behind the keys', async () => {
    await renderPane()
    const editor = await openReadme()
    fireEvent.change(editor, { target: { value: 'typed just now' } })
    expect(screen.getByTestId('editor-highlight')).toHaveTextContent('typed just now')
  })

  it('finds in the text being edited, counts matches, and cycles with Enter', async () => {
    await renderPane()
    const editor = await openReadme()

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Find in file'))
    })
    const input = await screen.findByPlaceholderText('Find in file')
    await act(async () => {
      fireEvent.change(input, { target: { value: 'world' } })
    })
    expect(screen.getByText('1/2')).toBeInTheDocument()

    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' })
    })
    expect(screen.getByText('2/2')).toBeInTheDocument()

    // The count follows the buffer, not the file on disk.
    await act(async () => {
      fireEvent.change(editor, { target: { value: `${CONTENT}\nworld once more` } })
    })
    expect(screen.getByText('2/3')).toBeInTheDocument()

    await act(async () => {
      fireEvent.keyDown(input, { key: 'Escape' })
    })
    expect(screen.queryByPlaceholderText('Find in file')).not.toBeInTheDocument()
  })

  it('saves with the keyboard, and keeps the chord from reaching the window', async () => {
    await renderPane()
    const editor = await openReadme()
    const reachedWindow = vi.fn()
    window.addEventListener('keydown', reachedWindow)

    await act(async () => {
      fireEvent.change(editor, { target: { value: 'hello\nchanged' } })
    })
    await act(async () => {
      fireEvent.keyDown(editor, { key: 's', metaKey: true })
    })
    window.removeEventListener('keydown', reachedWindow)

    await waitFor(() =>
      expect(mockWriteFileContent).toHaveBeenCalledWith(
        '/repo/README.md',
        'hello\nchanged',
        undefined
      )
    )
    // Elsewhere the same chord changes the view; here it must only save.
    expect(reachedWindow).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByLabelText(/^Save/)).not.toBeInTheDocument())
    expect(editor.value).toBe('hello\nchanged')
  })

  it('opens another file beside an edited one without asking', async () => {
    await renderPane()
    const editor = await openReadme()
    await act(async () => {
      fireEvent.change(editor, { target: { value: 'dirty content' } })
    })
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false)

    await act(async () => {
      fireEvent.click(screen.getByText('src'))
    })
    await act(async () => {
      fireEvent.click(await screen.findByText('index.ts'))
    })

    expect(confirmSpy).not.toHaveBeenCalled()
    expect(useAppStore.getState().filesPanes.get('t1')?.tabs).toEqual([
      '/repo/README.md',
      '/repo/src/index.ts'
    ])
    expect(editor.value).toBe('dirty content')
  })

  it('shows binary fallback when readFileContent returns null', async () => {
    mockReadFileContent.mockResolvedValueOnce(null)
    await renderPane()
    await act(async () => {
      fireEvent.click(screen.getByText('README.md'))
    })
    expect(await screen.findByText('Binary file — preview unavailable')).toBeInTheDocument()
  })

  it('says so when a file cannot be read at all, rather than calling it binary', async () => {
    mockReadFileContent.mockResolvedValueOnce(null)
    mockFileStamp.mockResolvedValue(null)
    await renderPane()
    await act(async () => {
      fireEvent.click(screen.getByText('README.md'))
    })
    expect(await screen.findByText('This file could not be read')).toBeInTheDocument()
  })

  it('opens a file too large to read whole as read-only, so its tail cannot be saved away', async () => {
    mockReadFileContent.mockResolvedValue(`first part${truncationMarker(900_000)}`)
    await renderPane()
    await act(async () => {
      fireEvent.click(screen.getByText('README.md'))
    })

    expect(await screen.findByText(/Too large to edit here/)).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: /^Edit / })).not.toBeInTheDocument()
    expect(screen.queryByLabelText(/^Save/)).not.toBeInTheDocument()
  })

  it('returns Empty directory when root has no entries', async () => {
    mockListDir.mockResolvedValue([])
    await act(async () => {
      render(<FilesCard sessionId="t1" />)
    })
    expect(await screen.findByText('Empty directory')).toBeInTheDocument()
  })

  it('persists the tree width to localStorage on pointerup', async () => {
    await renderPane()
    await openReadme()

    const divider = screen.getByRole('separator')
    const container = divider.parentElement as HTMLElement
    vi.spyOn(container, 'getBoundingClientRect').mockReturnValue({
      top: 0,
      left: 0,
      right: 1000,
      bottom: 400,
      width: 1000,
      height: 400,
      x: 0,
      y: 0,
      toJSON: () => ({})
    })

    await act(async () => {
      fireEvent.pointerDown(divider, { clientX: 380 })
    })
    await act(async () => {
      fireEvent(document, new PointerEvent('pointermove', { clientX: 300 }))
    })
    expect(localStorage.getItem('vorn:files-split-ratio')).toBeNull()

    await act(async () => {
      fireEvent(document, new PointerEvent('pointerup'))
    })
    const stored = localStorage.getItem('vorn:files-split-ratio')
    expect(stored).not.toBeNull()
    expect(Number(stored)).toBeCloseTo(0.3, 1)
  })
})
