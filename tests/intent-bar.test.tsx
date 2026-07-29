// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, cleanup, act, within } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Replace Node's experimental localStorage (which needs a file path)
// with an in-memory shim so getItem/setItem/clear work in tests.
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

const registryMocks = vi.hoisted(() => ({
  pasteToTerminal: vi.fn(),
  focusTerminal: vi.fn(),
  scrollToBottom: vi.fn()
}))

vi.mock('../src/renderer/lib/terminal-registry', () => registryMocks)

Object.defineProperty(window, 'api', {
  value: {
    writeTerminal: vi.fn()
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { IntentBar } from '../src/renderer/components/IntentBar'
import { focusIntentBar } from '../src/renderer/lib/intent-bar-focus'
import { recordCommand, resetCommandHistoryCache } from '../src/renderer/lib/command-history'
import { resetCompletionCaches } from '../src/renderer/lib/completions'

const writeTerminal = window.api.writeTerminal as ReturnType<typeof vi.fn>

const mockTerminal = {
  id: 'term-1',
  session: {
    id: 'term-1',
    agentType: 'shell' as const,
    projectName: 'Vorn',
    projectPath: '/tmp/vorn',
    status: 'running' as const,
    createdAt: Date.now(),
    pid: 1234
  },
  status: 'running' as const,
  lastOutputTimestamp: Date.now()
}

const initialState = useAppStore.getState()

function seedStore(overrides: Partial<ReturnType<typeof useAppStore.getState>> = {}) {
  act(() => {
    useAppStore.setState({
      ...initialState,
      terminals: new Map([['term-1', mockTerminal]]),
      ...overrides
    })
  })
}

function getInput(): HTMLTextAreaElement {
  // By role, not by placeholder: the placeholder changes with mode, and the
  // mode is what several of these tests are asserting on.
  return screen.getByRole('textbox') as HTMLTextAreaElement
}

describe('IntentBar', () => {
  beforeEach(() => {
    localStorage.clear()
    resetCommandHistoryCache()
    seedStore()
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  it('renders nothing for an unknown terminal', () => {
    const { container } = render(<IntentBar terminalId="nope" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('submits on Enter: pastes the text, sends CR, records history, clears input', () => {
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git status' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(registryMocks.pasteToTerminal).toHaveBeenCalledWith('term-1', 'git status')
    expect(writeTerminal).toHaveBeenCalledWith('term-1', '\r')
    expect(input.value).toBe('')
    expect(localStorage.getItem('vorn:commandHistory')).toContain('git status')
  })

  it('does not submit empty input', () => {
    render(<IntentBar terminalId="term-1" />)
    fireEvent.keyDown(getInput(), { key: 'Enter' })
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
    expect(writeTerminal).not.toHaveBeenCalled()
  })

  it('Shift+Enter does not submit', () => {
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'echo hi' } })
    fireEvent.keyDown(input, { key: 'Enter', shiftKey: true })
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('shows history suggestions while typing and runs the highlighted one', () => {
    recordCommand('yarn test', 'shell', '/tmp/vorn')
    recordCommand('yarn typecheck', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'yarn' } })

    expect(screen.getByText('yarn typecheck')).toBeInTheDocument()
    expect(screen.getByText('yarn test')).toBeInTheDocument()

    // Nothing highlighted yet: Enter would run the typed text. Navigate first.
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(registryMocks.pasteToTerminal).toHaveBeenCalledWith('term-1', 'yarn typecheck')
  })

  it('Enter without navigation submits the typed text, not a suggestion', () => {
    recordCommand('yarn typecheck', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'yarn' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(registryMocks.pasteToTerminal).toHaveBeenCalledWith('term-1', 'yarn')
  })

  it('Tab inserts the highlighted suggestion without running it', () => {
    recordCommand('yarn typecheck', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'yarn' } })
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    fireEvent.keyDown(input, { key: 'Tab' })

    expect(input.value).toBe('yarn typecheck')
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('ArrowUp on empty input opens recent history', () => {
    recordCommand('git status', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    expect(screen.getByText('git status')).toBeInTheDocument()
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(registryMocks.pasteToTerminal).toHaveBeenCalledWith('term-1', 'git status')
  })

  it('renders a ghost completion and accepts it with Tab', () => {
    recordCommand('git push origin main', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git pu' } })

    expect(screen.getByText('sh origin main')).toBeInTheDocument()

    // No dropdown highlight: Tab accepts the ghost into the editor.
    fireEvent.keyDown(input, { key: 'Tab' })
    expect(input.value).toBe('git push origin main')
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('Escape closes the dropdown first, then returns focus to the terminal', () => {
    recordCommand('git status', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git' } })
    expect(screen.getByText('git status')).toBeInTheDocument()

    fireEvent.keyDown(input, { key: 'Escape' })
    expect(screen.queryByText('git status')).not.toBeInTheDocument()
    expect(registryMocks.focusTerminal).not.toHaveBeenCalled()

    fireEvent.keyDown(input, { key: 'Escape' })
    expect(registryMocks.focusTerminal).toHaveBeenCalledWith('term-1')
  })

  it('renders nothing for agent sessions — their TUI owns input', () => {
    seedStore({
      terminals: new Map([
        [
          'term-1',
          {
            ...mockTerminal,
            session: { ...mockTerminal.session, agentType: 'claude' as const }
          }
        ]
      ])
    })
    const { container } = render(<IntentBar terminalId="term-1" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('focusIntentBar focuses the mounted composer input', () => {
    render(<IntentBar terminalId="term-1" />)
    expect(focusIntentBar('term-1')).toBe(true)
    expect(document.activeElement).toBe(getInput())
    expect(focusIntentBar('missing')).toBe(false)
  })
})

describe('IntentBar completions', () => {
  beforeEach(() => {
    localStorage.clear()
    resetCommandHistoryCache()
    resetCompletionCaches()
    Object.defineProperty(window, 'api', {
      value: {
        writeTerminal: vi.fn(),
        listShellExecutables: vi.fn().mockResolvedValue(['git', 'grep', 'go']),
        listDir: vi
          .fn()
          .mockResolvedValue([{ name: 'src', path: '/tmp/vorn/src', isDirectory: true }]),
        listBranches: vi
          .fn()
          .mockResolvedValue({ local: ['main', 'feat/x'], current: 'main', isGitRepo: true }),
        readFileContent: vi.fn().mockResolvedValue(JSON.stringify({ scripts: { dev: 'vite' } }))
      },
      writable: true
    })
    seedStore()
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  it('shows command completions with details while typing', async () => {
    render(<IntentBar terminalId="term-1" />)
    fireEvent.change(getInput(), { target: { value: 'g' } })
    expect(await screen.findByText('git')).toBeInTheDocument()
    expect(screen.getByText('version control')).toBeInTheDocument()
    expect(screen.getByText('grep')).toBeInTheDocument()
  })

  it('completes branches after git checkout', async () => {
    render(<IntentBar terminalId="term-1" />)
    fireEvent.change(getInput(), { target: { value: 'git checkout f' } })
    expect(await screen.findByText('feat/x')).toBeInTheDocument()
  })

  it('Enter on a highlighted completion inserts it without running', async () => {
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git che' } })
    expect(await screen.findByText('checkout')).toBeInTheDocument()

    fireEvent.keyDown(input, { key: 'ArrowUp' })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(input.value).toBe('git checkout ')
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('Tab without a highlight applies the first completion', async () => {
    render(<IntentBar terminalId="term-1" />)
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git che' } })
    expect(await screen.findByText('checkout')).toBeInTheDocument()

    fireEvent.keyDown(input, { key: 'Tab' })
    expect(input.value).toBe('git checkout ')
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('blends history rows above completions', async () => {
    recordCommand('git status', 'shell', '/tmp/vorn')
    render(<IntentBar terminalId="term-1" />)
    fireEvent.change(getInput(), { target: { value: 'g' } })
    expect(await screen.findByText('grep')).toBeInTheDocument()
    // Scoped to the suggestion list: the bar itself also renders a button
    // (the mode toggle), which is not a suggestion row.
    const rows = within(screen.getByRole('listbox'))
      .getAllByRole('button')
      .map((b) => b.textContent)
    expect(rows[0]).toContain('git status')
  })
})

describe('IntentBar intent modes', () => {
  const createTerminal = vi.fn()

  beforeEach(() => {
    localStorage.clear()
    resetCommandHistoryCache()
    resetCompletionCaches()
    createTerminal.mockReset()
    createTerminal.mockResolvedValue({
      id: 'agent-1',
      agentType: 'claude',
      projectName: 'vorn',
      projectPath: '/tmp/vorn',
      status: 'running',
      createdAt: 0,
      pid: 2
    })
    Object.defineProperty(window, 'api', {
      value: {
        writeTerminal: vi.fn(),
        notifyWidgetStatus: vi.fn(),
        createTerminal: (...args: unknown[]) => createTerminal(...args),
        listShellExecutables: vi.fn().mockResolvedValue(['git', 'yarn', 'ls']),
        listDir: vi.fn().mockResolvedValue([]),
        listBranches: vi.fn().mockResolvedValue({ local: [], current: 'main', isGitRepo: true }),
        readFileContent: vi.fn().mockResolvedValue(null),
        detectInstalledAgents: vi.fn().mockResolvedValue({
          claude: true,
          codex: true,
          copilot: false,
          opencode: false,
          gemini: false
        })
      },
      writable: true
    })
    seedStore()
  })

  /** The known-command set loads asynchronously; let it settle. */
  async function ready() {
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
  }

  it('offers both kinds of input before there is anything to classify', async () => {
    // Empty resolves to command mode, but a prompt is equally accepted and the
    // placeholder is the only thing that says so.
    render(<IntentBar terminalId="term-1" />)
    await ready()
    expect(getInput().placeholder).toBe('Type a command or send a prompt for the agent')
  })

  it('names only the pinned mode once the choice has been made', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    fireEvent.keyDown(getInput(), { key: 'i', metaKey: true })
    expect(getInput().placeholder).toBe('Describe a task')
  })

  it('starts in command mode', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    expect(screen.getByLabelText('Type a command')).toBeInTheDocument()
  })

  it('switches to prompt mode when the input is not a command', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    fireEvent.change(getInput(), { target: { value: 'why did the test fail' } })
    expect(screen.getByLabelText('Describe a task')).toBeInTheDocument()
  })

  it('stays in command mode for a known command', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    fireEvent.change(getInput(), { target: { value: 'git how do I rebase' } })
    expect(screen.getByLabelText('Type a command')).toBeInTheDocument()
  })

  it('Cmd+I pins the other mode, and Escape releases it', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git status' } })
    fireEvent.keyDown(input, { key: 'i', metaKey: true })
    expect(screen.getByLabelText('Describe a task')).toBeInTheDocument()

    fireEvent.keyDown(input, { key: 'Escape' })
    expect(screen.getByLabelText('Type a command')).toBeInTheDocument()
    // Releasing the pin must not also throw focus back to the terminal.
    expect(registryMocks.focusTerminal).not.toHaveBeenCalled()
  })

  it('launches an agent instead of writing to the pty in prompt mode', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    const input = getInput()
    fireEvent.change(input, { target: { value: 'add a test for the spine' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await act(async () => {
      await Promise.resolve()
    })
    expect(createTerminal).toHaveBeenCalledWith(
      expect.objectContaining({
        agentType: 'claude',
        projectPath: '/tmp/vorn',
        initialPrompt: 'add a test for the spine'
      })
    )
    expect(registryMocks.pasteToTerminal).not.toHaveBeenCalled()
  })

  it('still writes commands to the pty', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    const input = getInput()
    fireEvent.change(input, { target: { value: 'git status' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(registryMocks.pasteToTerminal).toHaveBeenCalledWith('term-1', 'git status')
    expect(createTerminal).not.toHaveBeenCalled()
  })

  it('offers no token completions for prose', async () => {
    render(<IntentBar terminalId="term-1" />)
    await ready()
    fireEvent.change(getInput(), { target: { value: 'explain the spine layout' } })
    await act(async () => {
      await new Promise((r) => setTimeout(r, 120))
    })
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })
})
