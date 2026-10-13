// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { OptionRow } from '../packages/web/src/renderer/components/OptionRow'

describe('OptionRow', () => {
  it('renders label text', () => {
    render(<OptionRow selected={false} label="Running" onClick={() => {}} />)
    expect(screen.getByText('Running')).toBeInTheDocument()
  })

  it('shows checkmark SVG when selected', () => {
    const { container } = render(<OptionRow selected={true} label="Running" onClick={() => {}} />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
    expect(svg?.querySelector('polyline')).toBeInTheDocument()
  })

  it('shows spacer span when not selected', () => {
    const { container } = render(<OptionRow selected={false} label="Running" onClick={() => {}} />)
    const svg = container.querySelector('svg')
    expect(svg).not.toBeInTheDocument()
    const spacer = container.querySelector('span.w-\\[11px\\]')
    expect(spacer).toBeInTheDocument()
  })

  it('renders colored dot when dot prop provided', () => {
    const { container } = render(
      <OptionRow selected={false} label="Running" dot="bg-green-500" onClick={() => {}} />
    )
    const dot = container.querySelector('.bg-green-500')
    expect(dot).toBeInTheDocument()
  })

  it('does not render dot when dot prop omitted', () => {
    const { container } = render(<OptionRow selected={false} label="Running" onClick={() => {}} />)
    const dot = container.querySelector('.rounded-full')
    expect(dot).not.toBeInTheDocument()
  })

  it('calls onClick on click', () => {
    const handleClick = vi.fn()
    render(<OptionRow selected={false} label="Running" onClick={handleClick} />)
    fireEvent.click(screen.getByText('Running'))
    expect(handleClick).toHaveBeenCalledTimes(1)
  })
})
