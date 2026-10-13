// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { Tooltip } from '../packages/web/src/renderer/components/Tooltip'

function getWrapper() {
  return screen.getByText('Hover me').parentElement!
}

describe('Tooltip', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('renders children', () => {
    render(
      <Tooltip label="Help text">
        <button>Hover me</button>
      </Tooltip>
    )
    expect(screen.getByText('Hover me')).toBeInTheDocument()
  })

  it('does not show tooltip initially', () => {
    render(
      <Tooltip label="Help text">
        <button>Hover me</button>
      </Tooltip>
    )
    expect(screen.queryByText('Help text')).not.toBeInTheDocument()
  })

  it('shows tooltip after hover + delay', () => {
    render(
      <Tooltip label="Help text" delay={200}>
        <button>Hover me</button>
      </Tooltip>
    )

    fireEvent.mouseEnter(getWrapper())
    act(() => vi.advanceTimersByTime(200))

    expect(screen.getByText('Help text')).toBeInTheDocument()
  })

  it('hides tooltip on mouse leave', () => {
    render(
      <Tooltip label="Help text" delay={0}>
        <button>Hover me</button>
      </Tooltip>
    )

    fireEvent.mouseEnter(getWrapper())
    act(() => vi.advanceTimersByTime(0))
    expect(screen.getByText('Help text')).toBeInTheDocument()

    fireEvent.mouseLeave(getWrapper())
    expect(screen.queryByText('Help text')).not.toBeInTheDocument()
  })

  it('does not show tooltip if mouse leaves before delay', () => {
    render(
      <Tooltip label="Help text" delay={500}>
        <button>Hover me</button>
      </Tooltip>
    )

    fireEvent.mouseEnter(getWrapper())
    fireEvent.mouseLeave(getWrapper())
    act(() => vi.advanceTimersByTime(500))

    expect(screen.queryByText('Help text')).not.toBeInTheDocument()
  })
})
