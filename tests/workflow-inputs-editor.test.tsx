// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import type { WorkflowInputDef } from '../src/shared/types'

// Exposes the option list so type-change behavior can be driven directly.
vi.mock('../src/renderer/components/SelectPicker', () => ({
  SelectPicker: (props: Record<string, unknown>) => (
    <select
      data-testid="type-picker"
      value={props.value as string}
      onChange={(e) => (props.onChange as (v: string) => void)(e.target.value)}
    >
      {(props.options as { value: string; label: string }[]).map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  )
}))

const { WorkflowInputsEditor } =
  await import('../src/renderer/components/workflow-editor/panels/WorkflowInputsEditor')

function setup(inputs: WorkflowInputDef[]) {
  const onChange = vi.fn()
  const utils = render(<WorkflowInputsEditor inputs={inputs} onChange={onChange} />)
  return { ...utils, onChange }
}

const oneText: WorkflowInputDef[] = [{ key: 'pr', label: 'PR', type: 'text' }]

describe('WorkflowInputsEditor', () => {
  it('edits the label', () => {
    const { getByLabelText, onChange } = setup(oneText)
    fireEvent.change(getByLabelText('Input label'), { target: { value: 'PR number' } })
    expect(onChange).toHaveBeenCalledWith([{ key: 'pr', label: 'PR number', type: 'text' }])
  })

  it('normalizes an edited key so it stays a usable template identifier', () => {
    const { getByLabelText, onChange } = setup(oneText)
    fireEvent.change(getByLabelText('Input key'), { target: { value: '2 pr num!' } })
    expect(onChange).toHaveBeenCalledWith([{ key: '_2_pr_num_', label: 'PR', type: 'text' }])
  })

  it('flags a key that collides with another input', () => {
    // Colliding keys silently lose one value when the run dialog builds its
    // value map, and make {{inputs.pr}} ambiguous.
    const { getAllByLabelText, getAllByText } = setup([
      { key: 'pr', label: 'A', type: 'text' },
      { key: 'pr', label: 'B', type: 'text' }
    ])
    expect(getAllByText(/Duplicate key/)).toHaveLength(2)
    for (const field of getAllByLabelText('Input key')) {
      expect(field).toHaveAttribute('aria-invalid', 'true')
    }
  })

  it('leaves distinct keys unflagged', () => {
    const { queryByText, getAllByLabelText } = setup([
      { key: 'pr', label: 'A', type: 'text' },
      { key: 'repo', label: 'B', type: 'text' }
    ])
    expect(queryByText(/Duplicate key/)).not.toBeInTheDocument()
    for (const field of getAllByLabelText('Input key')) {
      expect(field).not.toHaveAttribute('aria-invalid')
    }
  })

  it('does not rewrite a key that is briefly a prefix of another while typing', () => {
    // Auto-renaming on collision would fight anyone typing a longer name.
    const { getAllByLabelText, onChange } = setup([
      { key: 'pr', label: 'A', type: 'text' },
      { key: 'x', label: 'B', type: 'text' }
    ])
    fireEvent.change(getAllByLabelText('Input key')[1], { target: { value: 'pr' } })
    expect(onChange).toHaveBeenCalledWith([
      { key: 'pr', label: 'A', type: 'text' },
      { key: 'pr', label: 'B', type: 'text' }
    ])
  })

  it('removes the right input', () => {
    const { getByLabelText, onChange } = setup([
      ...oneText,
      { key: 'env', label: 'Env', type: 'text' }
    ])
    fireEvent.click(getByLabelText('Remove input env'))
    expect(onChange).toHaveBeenCalledWith(oneText)
  })

  it('toggles required, dropping the flag entirely when unchecked', () => {
    const { container, onChange } = setup([{ ...oneText[0], required: true }])
    const box = container.querySelector('input[type="checkbox"]')!
    expect(box).toBeChecked()
    fireEvent.click(box)
    // `undefined` rather than `false` keeps the stored workflow JSON clean.
    expect(onChange).toHaveBeenCalledWith([{ key: 'pr', label: 'PR', type: 'text' }])
  })

  it('seeds an options list when switching to select', () => {
    const { getByTestId, onChange } = setup(oneText)
    fireEvent.change(getByTestId('type-picker'), { target: { value: 'select' } })
    expect(onChange).toHaveBeenCalledWith([{ key: 'pr', label: 'PR', type: 'select', options: [] }])
  })

  it('drops stale options when switching away from select', () => {
    const { getByTestId, onChange } = setup([
      { key: 'e', label: 'E', type: 'select', options: [{ value: 'a', label: 'a' }] }
    ])
    fireEvent.change(getByTestId('type-picker'), { target: { value: 'text' } })
    expect(onChange).toHaveBeenCalledWith([
      { key: 'e', label: 'E', type: 'text', options: undefined }
    ])
  })

  it('parses comma-separated choices, ignoring blanks and padding', () => {
    const { getByLabelText, onChange } = setup([{ key: 'e', label: 'E', type: 'select' }])
    fireEvent.change(getByLabelText('Input choices'), { target: { value: 'dev, , prod ,' } })
    expect(onChange).toHaveBeenCalledWith([
      {
        key: 'e',
        label: 'E',
        type: 'select',
        options: [
          { value: 'dev', label: 'dev' },
          { value: 'prod', label: 'prod' }
        ]
      }
    ])
  })

  it('shows existing choices back as a comma-separated string', () => {
    const { getByLabelText } = setup([
      {
        key: 'e',
        label: 'E',
        type: 'select',
        options: [
          { value: 'dev', label: 'dev' },
          { value: 'prod', label: 'prod' }
        ]
      }
    ])
    expect(getByLabelText('Input choices')).toHaveValue('dev, prod')
  })

  it('edits the default value, clearing it to undefined when emptied', () => {
    const { getByLabelText, onChange } = setup([{ ...oneText[0], defaultValue: 'x' }])
    fireEvent.change(getByLabelText('Input default value'), { target: { value: '' } })
    expect(onChange).toHaveBeenCalledWith([{ key: 'pr', label: 'PR', type: 'text' }])
  })

  it('offers no choices or default field for a boolean', () => {
    const { queryByLabelText } = setup([{ key: 'f', label: 'F', type: 'boolean' }])
    expect(queryByLabelText('Input choices')).not.toBeInTheDocument()
    // A toggle's default is expressed by `required`/unchecked, not free text.
    expect(queryByLabelText('Input default value')).not.toBeInTheDocument()
  })
})
