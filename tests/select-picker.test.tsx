// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { SelectPicker } from '../packages/web/src/renderer/components/SelectPicker'

const OPTIONS = [
  { value: 'a', label: 'Apple' },
  { value: 'b', label: 'Banana' },
  { value: 'c', label: 'Cherry' }
]

describe('SelectPicker', () => {
  it('renders the current option label', () => {
    render(<SelectPicker value="b" options={OPTIONS} onChange={vi.fn()} />)
    expect(screen.getByText('Banana')).toBeInTheDocument()
  })

  it('shows placeholder when value matches no option', () => {
    render(
      <SelectPicker value="" options={OPTIONS} onChange={vi.fn()} placeholder="Pick fruit..." />
    )
    expect(screen.getByText('Pick fruit...')).toBeInTheDocument()
  })

  it('opens the popover with all options when the trigger is clicked', () => {
    render(<SelectPicker value="a" options={OPTIONS} onChange={vi.fn()} />)
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getAllByText('Apple').length).toBeGreaterThan(0)
    expect(screen.getByText('Banana')).toBeInTheDocument()
    expect(screen.getByText('Cherry')).toBeInTheDocument()
  })

  it('calls onChange when a different option is selected', () => {
    const onChange = vi.fn()
    render(<SelectPicker value="a" options={OPTIONS} onChange={onChange} />)
    fireEvent.click(screen.getByRole('button'))
    fireEvent.mouseDown(screen.getByText('Cherry'))
    expect(onChange).toHaveBeenCalledWith('c')
  })

  it('does not call onChange when the same option is re-selected', () => {
    const onChange = vi.fn()
    render(<SelectPicker value="a" options={OPTIONS} onChange={onChange} />)
    fireEvent.click(screen.getByRole('button'))
    fireEvent.mouseDown(screen.getAllByText('Apple')[1])
    expect(onChange).not.toHaveBeenCalled()
  })
})
