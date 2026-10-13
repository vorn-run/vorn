// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import React from 'react'

// jsdom doesn't implement matchMedia; stub it for useIsMobile.
Object.defineProperty(window, 'matchMedia', {
  value: () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn()
  }),
  writable: true
})

// Framer Motion's AnimatePresence keeps exiting elements in the DOM during
// the exit transition, which breaks synchronous dismissal assertions. Mock
// the library to be a transparent pass-through for these tests.
vi.mock('framer-motion', () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
  motion: new Proxy(
    {},
    {
      get: (_, tag: string) =>
        React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement>>((props, ref) =>
          React.createElement(tag, { ...props, ref })
        )
    }
  )
}))

import { toast, ToastContainer } from '../packages/web/src/renderer/components/Toast'

describe('toast module', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    // Flush any pending timers before restoring real timers.
    act(() => {
      vi.runAllTimers()
    })
    vi.useRealTimers()
  })

  it('toast() returns a string id', () => {
    const id = toast('hello', 'success')
    expect(typeof id).toBe('string')
    expect(id.length).toBeGreaterThan(0)
  })

  it('toast.loading() creates a sticky toast that does not auto-dismiss', () => {
    render(<ToastContainer />)
    act(() => {
      toast.loading('Working…')
    })
    expect(screen.getByText('Working…')).toBeInTheDocument()
    // Fast-forward far beyond any reasonable duration
    act(() => {
      vi.advanceTimersByTime(60_000)
    })
    expect(screen.getByText('Working…')).toBeInTheDocument()
    // Cleanup: dismiss so the module state doesn't leak into other tests
    act(() => {
      toast.dismiss('cleanup-id-does-not-exist')
    })
  })

  it('toast.success shortcut creates a success toast that dismisses after default duration', () => {
    render(<ToastContainer />)
    act(() => {
      toast.success('Saved!')
    })
    expect(screen.getByText('Saved!')).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(2500)
    })
    expect(screen.queryByText('Saved!')).not.toBeInTheDocument()
  })

  it('toast.error shortcut uses a longer duration (4000ms)', () => {
    render(<ToastContainer />)
    act(() => {
      toast.error('Boom')
    })
    expect(screen.getByText('Boom')).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(2500)
    })
    expect(screen.getByText('Boom')).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(1500)
    })
    expect(screen.queryByText('Boom')).not.toBeInTheDocument()
  })

  it('toast.warning and toast.info shortcuts render their messages', () => {
    render(<ToastContainer />)
    act(() => {
      toast.warning('Careful')
      toast.info('FYI')
    })
    expect(screen.getByText('Careful')).toBeInTheDocument()
    expect(screen.getByText('FYI')).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(4000)
    })
  })

  it('toast.update() mutates an existing toast message in place', () => {
    render(<ToastContainer />)
    let id = ''
    act(() => {
      id = toast.loading('Starting…')
    })
    expect(screen.getByText('Starting…')).toBeInTheDocument()
    act(() => {
      toast.update(id, 'Done!', 'success')
    })
    expect(screen.queryByText('Starting…')).not.toBeInTheDocument()
    expect(screen.getByText('Done!')).toBeInTheDocument()
    // Still auto-dismisses at the success duration
    act(() => {
      vi.advanceTimersByTime(2500)
    })
    expect(screen.queryByText('Done!')).not.toBeInTheDocument()
  })

  it('toast.update() on an unknown id falls back to creating a fresh toast', () => {
    render(<ToastContainer />)
    act(() => {
      toast.update('does-not-exist', 'Fallback', 'success')
    })
    expect(screen.getByText('Fallback')).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(2500)
    })
  })

  it('toast.dismiss() removes a toast immediately', () => {
    render(<ToastContainer />)
    let id = ''
    act(() => {
      id = toast.loading('Waiting…')
    })
    expect(screen.getByText('Waiting…')).toBeInTheDocument()
    act(() => {
      toast.dismiss(id)
    })
    expect(screen.queryByText('Waiting…')).not.toBeInTheDocument()
  })

  it('calling toast.update twice in quick succession does not prematurely dismiss from the first timer', () => {
    // This is the bug fix — previously the first update's setTimeout could fire
    // and dismiss the toast even after a second update scheduled a longer timer.
    render(<ToastContainer />)
    let id = ''
    act(() => {
      id = toast.loading('Step 1')
    })
    act(() => {
      toast.update(id, 'Step 2', 'success') // duration 2500
    })
    act(() => {
      vi.advanceTimersByTime(1000)
    })
    act(() => {
      toast.update(id, 'Step 3', 'success') // new duration 2500 from now (3500 total)
    })
    // At t=2500 (when the FIRST timer would have fired), the toast must still be visible.
    act(() => {
      vi.advanceTimersByTime(1500) // now t=2500
    })
    expect(screen.getByText('Step 3')).toBeInTheDocument()
    // Confirm the new timer still dismisses at the expected time.
    act(() => {
      vi.advanceTimersByTime(1000) // now t=3500
    })
    expect(screen.queryByText('Step 3')).not.toBeInTheDocument()
  })

  it('clicking the toast close button dismisses it and clears its timer', () => {
    render(<ToastContainer />)
    act(() => {
      toast.success('Click me off')
    })
    expect(screen.getByText('Click me off')).toBeInTheDocument()
    const closeButtons = screen.getAllByRole('button')
    act(() => {
      fireEvent.click(closeButtons[closeButtons.length - 1])
    })
    expect(screen.queryByText('Click me off')).not.toBeInTheDocument()
  })

  it('renders a spinner on loading toasts', () => {
    const { container } = render(<ToastContainer />)
    act(() => {
      toast.loading('Spinning')
    })
    expect(container.querySelector('.animate-spin')).toBeInTheDocument()
  })
})
