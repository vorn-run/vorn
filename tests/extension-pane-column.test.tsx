// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/components/FilesCard', () => ({
  FilesCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`files-${sessionId}`} />
}))
vi.mock('../src/renderer/components/EditorCard', () => ({
  EditorCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`editor-${sessionId}`} />
}))
vi.mock('../src/renderer/components/BrowserCard', () => ({
  BrowserCard: ({ sessionId }: { sessionId: string }) => (
    <div data-testid={`browser-${sessionId}`} />
  )
}))
vi.mock('../src/renderer/components/DeviceCard', () => ({
  DeviceCard: ({ sessionId }: { sessionId: string }) => <div data-testid={`device-${sessionId}`} />
}))
vi.mock('../src/renderer/components/TerminalsCard', () => ({
  TerminalsCard: ({ sessionId }: { sessionId: string }) => (
    <div data-testid={`terminals-${sessionId}`} />
  )
}))
// The pty behind a program pane is real; what matters here is that the pane
// draws it from the id the host handed back, with no session to look up.
vi.mock('../src/renderer/components/TerminalPane', () => ({
  TerminalPane: ({ terminalId }: { terminalId: string }) => (
    <div data-testid={`terminal-pane-${terminalId}`} />
  )
}))

import { useAppStore } from '../src/renderer/stores'
import { PaneColumn } from '../src/renderer/components/PaneColumn'
import { extensionPaneId } from '../src/renderer/lib/pane-id'
import type { ExtensionOpenPane } from '../packages/shared/src/types'

const closePane = vi.fn(async () => ({ closed: true }))

const opened = (over: Partial<ExtensionOpenPane> = {}): ExtensionOpenPane => ({
  extensionId: 'demo',
  paneId: 'report',
  sessionId: 't1',
  url: 'http://127.0.0.1:6000/extensions/demo/pane/report/n0/',
  nonce: 'n0',
  ...over
})

function seed(open: ExtensionOpenPane | null, others: { device?: boolean } = {}): void {
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        [
          't1',
          {
            id: 't1',
            session: { id: 't1', agentType: 'claude', projectName: 'p', projectPath: '/p' },
            status: 'idle',
            lastOutputTimestamp: 1
          }
        ]
      ]) as never,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      devicePanes: others.device
        ? (new Map([['t1', { udid: 'u', name: 'iPhone' }]]) as never)
        : new Map(),
      terminalsPanes: new Map(),
      extensionPanes: new Map(
        open ? [['t1', { open, title: 'Report', extensionName: 'Demo' }]] : []
      ),
      cardSplits: {},
      maximizedPaneId: null,
      config: null
    })
  })
}

describe('an extension pane in a session column', () => {
  beforeEach(() => {
    ;(window as unknown as { api: Record<string, unknown> }).api = { closeExtensionPane: closePane }
    closePane.mockClear()
  })

  afterEach(() => cleanup())

  it('draws no column at all while no extension has opened one', () => {
    seed(null)
    const { container } = render(<PaneColumn sessionId="t1" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('sits after the device pane, which is where the menu lists it', () => {
    seed(opened(), { device: true })
    render(<PaneColumn sessionId="t1" />)

    const drawn = [...document.querySelectorAll('[data-testid]')]
      .map((el) => el.getAttribute('data-testid'))
      .filter((id) => id?.startsWith('device-') || id?.startsWith('extension-pane-header-'))
    expect(drawn).toEqual(['device-t1', 'extension-pane-header-t1'])
  })

  // The page is framed, not fetched: the grant proving it is in the path the
  // host handed back, so the frame is built from that URL and not assembled here.
  it('frames the page the host granted, sandboxed and told to send no referrer', () => {
    seed(opened())
    render(<PaneColumn sessionId="t1" />)

    const frame = screen.getByTestId('extension-pane-frame-t1')
    expect(frame).toHaveAttribute('src', 'http://127.0.0.1:6000/extensions/demo/pane/report/n0/')
    expect(frame).toHaveAttribute('sandbox', 'allow-scripts allow-same-origin allow-forms')
    expect(frame).toHaveAttribute('referrerPolicy', 'no-referrer')
    // Nothing that would let a pane replace the window it sits in.
    expect(frame.getAttribute('sandbox')).not.toContain('allow-top-navigation')
    expect(frame.getAttribute('sandbox')).not.toContain('allow-popups')
  })

  it('draws a program pane as the terminal it already is', () => {
    seed(opened({ paneId: 'top', url: undefined, terminalId: 'pty-1' }))
    render(<PaneColumn sessionId="t1" />)

    expect(screen.getByTestId('terminal-pane-pty-1')).toBeInTheDocument()
    expect(document.querySelector('[data-testid="extension-pane-frame-t1"]')).toBeNull()
  })

  it('says which extension put it there, beside what it is called', () => {
    seed(opened())
    render(<PaneColumn sessionId="t1" />)

    expect(screen.getByText('Report')).toBeInTheDocument()
    expect(screen.getByText('Demo')).toBeInTheDocument()
  })

  it('maximizes and closes from its own bar, and offers no minimize', () => {
    seed(opened())
    render(<PaneColumn sessionId="t1" />)

    fireEvent.click(screen.getByLabelText('Maximize Report'))
    expect(useAppStore.getState().maximizedPaneId).toBe(extensionPaneId('t1'))

    // A pane inside a card is not a dock cell, so there is nowhere to minimize to.
    expect(document.querySelector('[aria-label="Minimize Report"]')).toBeNull()

    fireEvent.click(screen.getByLabelText('Close Report'))
    expect(closePane).toHaveBeenCalledWith('n0')
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
  })
})
