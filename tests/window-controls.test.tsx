// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const windowMinimize = vi.fn()
const windowMaximize = vi.fn()
const windowClose = vi.fn()
const isWindowMaximized = vi.fn().mockResolvedValue(false)
// Takes the listener it is given; the bare form infers a zero-argument call and
// no `mockImplementationOnce` that actually uses the callback will fit it.
const onWindowMaximizedChange = vi.fn((_cb: (maximized: boolean) => void) => () => {})

Object.defineProperty(window, 'api', {
  value: {
    windowMinimize,
    windowMaximize,
    windowClose,
    isWindowMaximized,
    onWindowMaximizedChange,
    getAppVersion: () => 'test',
    detectIDEs: vi.fn().mockResolvedValue([])
  },
  writable: true
})

// Force non-Mac + non-Web so WindowControls renders
vi.mock('../src/renderer/lib/platform', () => ({
  isMac: false,
  isWeb: false
}))

import { WindowControls } from '../src/renderer/components/WindowControls'

beforeEach(() => {
  windowMinimize.mockClear()
  windowMaximize.mockClear()
  windowClose.mockClear()
})

describe('WindowControls', () => {
  it('renders minimize, maximize, and close buttons', () => {
    render(<WindowControls />)
    expect(screen.getByRole('button', { name: 'Minimize' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Maximize' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument()
  })

  it('calls window.api.windowMinimize on minimize click', () => {
    render(<WindowControls />)
    fireEvent.click(screen.getByRole('button', { name: 'Minimize' }))
    expect(windowMinimize).toHaveBeenCalledTimes(1)
  })

  it('calls window.api.windowMaximize on maximize click', () => {
    render(<WindowControls />)
    fireEvent.click(screen.getByRole('button', { name: 'Maximize' }))
    expect(windowMaximize).toHaveBeenCalledTimes(1)
  })

  it('calls window.api.windowClose on close click', () => {
    render(<WindowControls />)
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(windowClose).toHaveBeenCalledTimes(1)
  })

  it('shows Restore label when the window is already maximized on mount', async () => {
    isWindowMaximized.mockResolvedValueOnce(true)
    render(<WindowControls />)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument())
  })

  it('flips the glyph when the main process reports a state change', async () => {
    let notify: ((maximized: boolean) => void) | undefined
    onWindowMaximizedChange.mockImplementationOnce((cb: (m: boolean) => void) => {
      notify = cb
      return () => {}
    })
    render(<WindowControls />)
    await waitFor(() => expect(notify).toBeDefined())
    expect(screen.getByRole('button', { name: 'Maximize' })).toBeInTheDocument()
    await act(async () => {
      notify?.(true)
    })
    expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument()
  })

  it('ignores initial isWindowMaximized() result when an event has already fired', async () => {
    let notify: ((maximized: boolean) => void) | undefined
    let resolveInitial: ((m: boolean) => void) | undefined
    isWindowMaximized.mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          resolveInitial = resolve
        })
    )
    onWindowMaximizedChange.mockImplementationOnce((cb: (m: boolean) => void) => {
      notify = cb
      return () => {}
    })
    render(<WindowControls />)
    await waitFor(() => expect(notify).toBeDefined())
    await act(async () => {
      notify?.(true)
    })
    expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument()
    await act(async () => {
      resolveInitial?.(false)
    })
    expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument()
  })
})
