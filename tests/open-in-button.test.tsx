// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../packages/web/src/renderer/assets/icons/vscode.svg?raw', () => ({ default: '<svg />' }))
vi.mock('../packages/web/src/renderer/assets/icons/cursor.svg?raw', () => ({ default: '<svg />' }))
vi.mock('../packages/web/src/renderer/assets/icons/windsurf.svg?raw', () => ({
  default: '<svg />'
}))
vi.mock('../packages/web/src/renderer/assets/icons/zed.svg?raw', () => ({ default: '<svg />' }))
vi.mock('../packages/web/src/renderer/assets/icons/sublime.svg?raw', () => ({ default: '<svg />' }))
vi.mock('../packages/web/src/renderer/assets/icons/webstorm.svg?raw', () => ({
  default: '<svg />'
}))
vi.mock('../packages/web/src/renderer/assets/icons/intellij.svg?raw', () => ({
  default: '<svg />'
}))
vi.mock('../packages/web/src/renderer/assets/icons/xcode.svg?raw', () => ({ default: '<svg />' }))
vi.mock('../packages/web/src/renderer/assets/icons/terminal.svg?raw', () => ({
  default: '<svg />'
}))
vi.mock('../packages/web/src/renderer/assets/icons/finder.svg?raw', () => ({ default: '<svg />' }))

const detectIDEs = vi.fn()
const openInIDE = vi.fn()

Object.defineProperty(window, 'api', {
  value: { detectIDEs, openInIDE },
  writable: true
})

beforeEach(() => {
  vi.resetModules()
  detectIDEs.mockReset()
  openInIDE.mockReset()
})

afterEach(() => {
  cleanup()
})

async function importButton() {
  const mod = await import('../packages/web/src/renderer/components/OpenInButton')
  return mod.OpenInButton
}

describe('OpenInButton', () => {
  it('renders nothing until detectIDEs resolves with at least one IDE', async () => {
    detectIDEs.mockResolvedValue([])
    const OpenInButton = await importButton()
    const { container } = render(<OpenInButton projectPath="/tmp/proj" />)
    await waitFor(() => expect(detectIDEs).toHaveBeenCalled())
    expect(container).toBeEmptyDOMElement()
  })

  it('renders the default IDE split button and calls openInIDE on primary click', async () => {
    detectIDEs.mockResolvedValue([
      { id: 'vscode', name: 'VS Code', command: 'code' },
      { id: 'cursor', name: 'Cursor', command: 'cursor' }
    ])
    const OpenInButton = await importButton()
    render(<OpenInButton projectPath="/tmp/proj" />)

    const primary = await screen.findByTitle('Open in VS Code')
    fireEvent.click(primary)
    expect(openInIDE).toHaveBeenCalledWith('vscode', '/tmp/proj')
  })

  it('toggles the dropdown menu and opens a non-default IDE when selected', async () => {
    detectIDEs.mockResolvedValue([
      { id: 'vscode', name: 'VS Code', command: 'code' },
      { id: 'cursor', name: 'Cursor', command: 'cursor' }
    ])
    const OpenInButton = await importButton()
    render(<OpenInButton projectPath="/tmp/proj" direction="up" />)

    const toggle = await screen.findByLabelText('Choose IDE')
    fireEvent.click(toggle)

    const cursorOption = await screen.findByRole('button', { name: /Cursor/ })
    fireEvent.click(cursorOption)
    expect(openInIDE).toHaveBeenCalledWith('cursor', '/tmp/proj')

    // Menu closes after selection
    await waitFor(() => expect(screen.queryByRole('button', { name: /Cursor/ })).toBeNull())
  })

  it('closes the menu on outside mousedown', async () => {
    detectIDEs.mockResolvedValue([{ id: 'vscode', name: 'VS Code', command: 'code' }])
    const OpenInButton = await importButton()
    render(<OpenInButton projectPath="/tmp/proj" />)

    const toggle = await screen.findByLabelText('Choose IDE')
    fireEvent.click(toggle)
    expect(await screen.findByText('Open in')).toBeInTheDocument()

    fireEvent.mouseDown(document.body)
    await waitFor(() => expect(screen.queryByText('Open in')).toBeNull())
  })

  it('retries detectIDEs after a rejection by clearing the in-flight cache', async () => {
    detectIDEs.mockRejectedValueOnce(new Error('rpc down'))
    const OpenInButton = await importButton()
    const { unmount } = render(<OpenInButton projectPath="/tmp/proj" />)
    await waitFor(() => expect(detectIDEs).toHaveBeenCalledTimes(1))
    unmount()

    detectIDEs.mockResolvedValueOnce([{ id: 'vscode', name: 'VS Code', command: 'code' }])
    render(<OpenInButton projectPath="/tmp/proj" />)
    await waitFor(() => expect(detectIDEs).toHaveBeenCalledTimes(2))
    expect(await screen.findByTitle('Open in VS Code')).toBeInTheDocument()
  })
})
