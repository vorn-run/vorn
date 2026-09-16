// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ApprovalConfigForm } from '../src/renderer/components/workflow-editor/panels/ApprovalConfigForm'
import type { ApprovalConfig, WorkflowNode } from '../src/shared/types'

describe('ApprovalConfigForm', () => {
  it('renders message textarea and timeout input', () => {
    const { container } = render(<ApprovalConfigForm config={{}} onChange={vi.fn()} />)
    expect(container.querySelector('textarea')).toBeTruthy()
    expect(container.querySelector('input[type="number"]')).toBeTruthy()
    expect(container.textContent).toContain('Message')
    expect(container.textContent).toContain('Timeout')
  })

  it('invokes onChange with new message text', () => {
    const onChange = vi.fn()
    const { container } = render(<ApprovalConfigForm config={{}} onChange={onChange} />)
    const textarea = container.querySelector('textarea')!
    fireEvent.change(textarea, { target: { value: 'Please review' } })
    expect(onChange).toHaveBeenCalledWith({ message: 'Please review' })
  })

  it('converts seconds input to timeoutMs on change', () => {
    const onChange = vi.fn()
    const { container } = render(<ApprovalConfigForm config={{}} onChange={onChange} />)
    const input = container.querySelector('input[type="number"]')!
    fireEvent.change(input, { target: { value: '30' } })
    expect(onChange).toHaveBeenCalledWith({ timeoutMs: 30000 })
  })

  it('clears timeoutMs when input is emptied', () => {
    const onChange = vi.fn()
    const config: ApprovalConfig = { timeoutMs: 5000 }
    const { container } = render(<ApprovalConfigForm config={config} onChange={onChange} />)
    const input = container.querySelector('input[type="number"]')!
    fireEvent.change(input, { target: { value: '' } })
    expect(onChange).toHaveBeenCalledWith({ timeoutMs: undefined })
  })

  it('displays existing timeout converted to seconds', () => {
    const config: ApprovalConfig = { timeoutMs: 60000 }
    const { container } = render(<ApprovalConfigForm config={config} onChange={vi.fn()} />)
    const input = container.querySelector('input[type="number"]') as HTMLInputElement
    expect(input.value).toBe('60')
  })

  describe('letting the reviewer request changes', () => {
    const steps = [
      { id: 'trigger', type: 'trigger', label: 'Every day at 11:00' },
      { id: 'humanize', type: 'launchAgent', label: 'Make it sound like a person' }
    ] as WorkflowNode[]

    it('turns on with the first step above the gate and three rounds', () => {
      const onChange = vi.fn()
      const { getByRole } = render(
        <ApprovalConfigForm config={{}} onChange={onChange} redoFromSteps={steps} slug="approve" />
      )
      fireEvent.click(getByRole('switch'))
      expect(onChange).toHaveBeenCalledWith({ feedback: { from: 'humanize', maxRounds: 3 } })
    })

    it('offers only steps above the gate, keeps rounds in bounds, and names the variable', () => {
      const onChange = vi.fn()
      const config: ApprovalConfig = { feedback: { from: 'humanize', maxRounds: 3 } }
      const { getByLabelText, container } = render(
        <ApprovalConfigForm
          config={config}
          onChange={onChange}
          redoFromSteps={steps}
          slug="approve"
        />
      )
      const select = getByLabelText('Redo from') as HTMLSelectElement
      expect([...select.options].map((o) => o.textContent)).toEqual(['Make it sound like a person'])
      fireEvent.change(getByLabelText('up to'), { target: { value: '40' } })
      expect(onChange).toHaveBeenLastCalledWith({ feedback: { from: 'humanize', maxRounds: 10 } })
      expect(container.textContent).toContain('{{steps.approve.feedback}}')
    })

    it('turns off without leaving the setting behind', () => {
      const onChange = vi.fn()
      const { getByRole } = render(
        <ApprovalConfigForm
          config={{ message: 'ok?', feedback: { from: 'humanize', maxRounds: 3 } }}
          onChange={onChange}
          redoFromSteps={steps}
        />
      )
      fireEvent.click(getByRole('switch'))
      expect(onChange).toHaveBeenCalledWith({ message: 'ok?' })
    })

    it('cannot turn on with no step above the gate to send work back to', () => {
      const { getByRole, container } = render(
        <ApprovalConfigForm config={{}} onChange={vi.fn()} redoFromSteps={[steps[0]]} />
      )
      expect(getByRole('switch')).toBeDisabled()
      expect(container.textContent).toContain('Add a step before this gate')
    })
  })

  describe('the text a reviewer may rewrite', () => {
    it('names the variable later steps read it through', () => {
      const { container } = render(
        <ApprovalConfigForm config={{}} onChange={vi.fn()} slug="approve" />
      )
      expect(container.textContent).toContain('Editable text')
      expect(container.textContent).toContain('{{steps.approve.text}}')
    })

    it('keeps the template, and clears the setting when it is emptied', () => {
      const onChange = vi.fn()
      const { container, rerender } = render(<ApprovalConfigForm config={{}} onChange={onChange} />)
      const field = container.querySelectorAll('textarea')[1]
      fireEvent.change(field, { target: { value: '{{steps.draft.output}}' } })
      expect(onChange).toHaveBeenCalledWith({ edit: '{{steps.draft.output}}' })

      rerender(
        <ApprovalConfigForm config={{ edit: '{{steps.draft.output}}' }} onChange={onChange} />
      )
      fireEvent.change(container.querySelectorAll('textarea')[1], { target: { value: '' } })
      expect(onChange).toHaveBeenLastCalledWith({ edit: undefined })
    })
  })

  it('rejects zero/negative timeout input', () => {
    const onChange = vi.fn()
    const { container } = render(<ApprovalConfigForm config={{}} onChange={onChange} />)
    const input = container.querySelector('input[type="number"]')!
    fireEvent.change(input, { target: { value: '0' } })
    expect(onChange).toHaveBeenCalledWith({ timeoutMs: undefined })
  })
})
