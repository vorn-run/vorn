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

describe('TriggerConfigForm — run inputs', () => {
  it('offers the inputs editor only for the manual trigger type', () => {
    const { unmount } = render(
      <TriggerConfigForm config={{ triggerType: 'manual' }} onChange={vi.fn()} />
    )
    expect(screen.getByText('Run Inputs')).toBeInTheDocument()
    unmount()

    render(<TriggerConfigForm config={{ triggerType: 'taskCreated' }} onChange={vi.fn()} />)
    expect(screen.queryByText('Run Inputs')).not.toBeInTheDocument()
  })

  it('adds an input with a usable default key', () => {
    const onChange = vi.fn()
    render(<TriggerConfigForm config={{ triggerType: 'manual' }} onChange={onChange} />)
    fireEvent.click(screen.getByText('Add input'))

    expect(onChange).toHaveBeenCalledWith({
      triggerType: 'manual',
      inputs: [{ key: 'input', label: '', type: 'text' }]
    })
  })

  it('does not clobber the contextual flag when adding an input', () => {
    const onChange = vi.fn()
    render(
      <TriggerConfigForm config={{ triggerType: 'manual', contextual: true }} onChange={onChange} />
    )
    fireEvent.click(screen.getByText('Add input'))

    expect(onChange.mock.calls[0][0]).toMatchObject({ contextual: true })
  })

  it('normalizes a typed key so it survives the template syntax', () => {
    const onChange = vi.fn()
    render(
      <TriggerConfigForm
        config={{ triggerType: 'manual', inputs: [{ key: 'a', label: '', type: 'text' }] }}
        onChange={onChange}
      />
    )
    fireEvent.change(screen.getByLabelText('Input key'), {
      target: { value: 'issue url.x' }
    })

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ inputs: [expect.objectContaining({ key: 'issue_url_x' })] })
    )
  })

  it('names a second input uniquely rather than colliding with the first', () => {
    const onChange = vi.fn()
    render(
      <TriggerConfigForm
        config={{ triggerType: 'manual', inputs: [{ key: 'input', label: '', type: 'text' }] }}
        onChange={onChange}
      />
    )
    fireEvent.click(screen.getByText('Add input'))

    expect(onChange.mock.calls[0][0].inputs[1].key).toBe('input_2')
  })

  it('drops the inputs key entirely when the last input is removed', () => {
    const onChange = vi.fn()
    render(
      <TriggerConfigForm
        config={{ triggerType: 'manual', inputs: [{ key: 'issue', label: '', type: 'text' }] }}
        onChange={onChange}
      />
    )
    fireEvent.click(screen.getByLabelText('Remove input issue'))

    expect(onChange).toHaveBeenCalledWith({ triggerType: 'manual', inputs: undefined })
  })
})
