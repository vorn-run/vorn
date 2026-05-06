// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { TriggerConfig } from '../src/shared/types'

vi.mock('../src/renderer/stores', () => {
  const state = { config: { projects: [] } }
  return {
    useAppStore: (selector?: (s: unknown) => unknown) => (selector ? selector(state) : state)
  }
})

const { TriggerConfigForm } =
  await import('../src/renderer/components/workflow-editor/panels/TriggerConfigForm')

describe('TriggerConfigForm', () => {
  it('renders the trigger type label and current type hint', () => {
    render(<TriggerConfigForm config={{ triggerType: 'manual' }} onChange={vi.fn()} />)
    expect(screen.getByText('Trigger Type')).toBeInTheDocument()
    expect(screen.getByText(/Run manually/)).toBeInTheDocument()
  })

  it('shows the Run At input for the once trigger type', () => {
    const config: TriggerConfig = { triggerType: 'once', runAt: new Date().toISOString() }
    render(<TriggerConfigForm config={config} onChange={vi.fn()} />)
    expect(screen.getByText('Run At')).toBeInTheDocument()
  })

  it('shows cron + timezone inputs for the recurring trigger type', () => {
    const config: TriggerConfig = { triggerType: 'recurring', cron: '0 9 * * *' }
    render(<TriggerConfigForm config={config} onChange={vi.fn()} />)
    expect(screen.getByText('Cron Expression')).toBeInTheDocument()
    expect(screen.getByText('Timezone')).toBeInTheDocument()
    expect(screen.getByText('Preset')).toBeInTheDocument()
  })

  it('updates the cron value when typed', () => {
    const onChange = vi.fn()
    const config: TriggerConfig = { triggerType: 'recurring', cron: '0 9 * * *' }
    render(<TriggerConfigForm config={config} onChange={onChange} />)
    const input = screen.getByPlaceholderText('* * * * *')
    fireEvent.change(input, { target: { value: '*/15 * * * *' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ cron: '*/15 * * * *' }))
  })

  it('shows project filter for the taskCreated trigger type', () => {
    render(<TriggerConfigForm config={{ triggerType: 'taskCreated' }} onChange={vi.fn()} />)
    expect(screen.getByText('Project Filter')).toBeInTheDocument()
  })

  it('shows From/To status pickers for the taskStatusChanged trigger type', () => {
    render(<TriggerConfigForm config={{ triggerType: 'taskStatusChanged' }} onChange={vi.fn()} />)
    expect(screen.getByText('From Status')).toBeInTheDocument()
    expect(screen.getByText('To Status')).toBeInTheDocument()
    expect(screen.getByText('Project Filter')).toBeInTheDocument()
  })

  it('updates timezone input', () => {
    const onChange = vi.fn()
    const config: TriggerConfig = { triggerType: 'recurring', cron: '0 9 * * *', timezone: 'UTC' }
    const { container } = render(<TriggerConfigForm config={config} onChange={onChange} />)
    const tzInput = container.querySelectorAll('input[type="text"]')[1] as HTMLInputElement
    fireEvent.change(tzInput, { target: { value: 'America/Los_Angeles' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ timezone: 'America/Los_Angeles' })
    )
  })

  it('updates the runAt input', () => {
    const onChange = vi.fn()
    const config: TriggerConfig = { triggerType: 'once', runAt: new Date().toISOString() }
    const { container } = render(<TriggerConfigForm config={config} onChange={onChange} />)
    const input = container.querySelector('input[type="datetime-local"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '2026-12-31T10:30' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ triggerType: 'once' }))
  })

  it('switches trigger type via the picker', () => {
    const onChange = vi.fn()
    render(<TriggerConfigForm config={{ triggerType: 'manual' }} onChange={onChange} />)
    fireEvent.click(screen.getByText('Manual'))
    fireEvent.mouseDown(screen.getByText('Recurring'))
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ triggerType: 'recurring' }))
  })
})

describe('TriggerConfigForm — contextual toggle', () => {
  it('renders the Contextual switch only for manual triggers', () => {
    const { container, rerender } = render(
      <TriggerConfigForm config={{ triggerType: 'manual' }} onChange={vi.fn()} />
    )
    expect(container.textContent).toContain('Contextual')

    rerender(
      <TriggerConfigForm
        config={{ triggerType: 'recurring', cron: '0 9 * * *' }}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).not.toContain('Contextual')
  })

  it('reflects the current contextual flag on the switch', () => {
    render(
      <TriggerConfigForm config={{ triggerType: 'manual', contextual: true }} onChange={vi.fn()} />
    )
    const sw = screen.getAllByRole('switch')[0]
    expect(sw.getAttribute('aria-checked')).toBe('true')
  })

  it('sets contextual: true when toggled from off', () => {
    const onChange = vi.fn()
    render(<TriggerConfigForm config={{ triggerType: 'manual' }} onChange={onChange} />)
    fireEvent.click(screen.getAllByRole('switch')[0])
    expect(onChange).toHaveBeenCalledWith({ triggerType: 'manual', contextual: true })
  })

  it('drops the contextual flag when toggled from on', () => {
    const onChange = vi.fn()
    render(
      <TriggerConfigForm config={{ triggerType: 'manual', contextual: true }} onChange={onChange} />
    )
    fireEvent.click(screen.getAllByRole('switch')[0])
    expect(onChange).toHaveBeenCalledWith({ triggerType: 'manual', contextual: undefined })
  })
})
