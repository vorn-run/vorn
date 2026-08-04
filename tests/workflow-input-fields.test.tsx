// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/components/ProjectPicker', () => ({
  ProjectPicker: (props: Record<string, unknown>) => (
    <button
      data-testid="project-picker"
      data-allow-none={String(props.allowNone)}
      data-current={String(props.currentProject)}
      onClick={() => (props.onChange as (v: string) => void)('Vorn')}
    />
  )
}))

vi.mock('../src/renderer/components/SelectPicker', () => ({
  SelectPicker: (props: Record<string, unknown>) => (
    <button
      data-testid="select-picker"
      data-current={String(props.value)}
      data-count={String((props.options as unknown[]).length)}
      onClick={() => (props.onChange as (v: string) => void)('b')}
    />
  )
}))

const mockState = { config: { projects: [{ name: 'Vorn', path: '/p' }] } }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (s: unknown) => unknown) => (selector ? selector(mockState) : mockState)
}))

const { WorkflowInputFields } = await import('../src/renderer/components/WorkflowInputFields')

type Def = Parameters<typeof WorkflowInputFields>[0]['defs'][number]

function renderField(def: Def, value: unknown = '') {
  const onChange = vi.fn()
  const utils = render(
    <WorkflowInputFields defs={[def]} values={{ [def.key]: value }} onChange={onChange} />
  )
  return { ...utils, onChange }
}

describe('WorkflowInputFields', () => {
  it('renders nothing when there are no declared inputs', () => {
    const { container } = render(<WorkflowInputFields defs={[]} values={{}} onChange={vi.fn()} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('falls back to the key when no label was given, and marks required fields', () => {
    const { getByText, getByLabelText } = renderField({
      key: 'pr_number',
      label: '',
      type: 'text',
      required: true
    })
    expect(getByLabelText('pr_number')).toBeInTheDocument()
    expect(getByText('*')).toBeInTheDocument()
  })

  it('renders a description under the control when one is declared', () => {
    const { getByText } = renderField({
      key: 'k',
      label: 'K',
      type: 'text',
      description: 'why this matters'
    })
    expect(getByText('why this matters')).toBeInTheDocument()
  })

  it('edits a text input', () => {
    const { getByLabelText, onChange } = renderField({ key: 'k', label: 'K', type: 'text' })
    fireEvent.change(getByLabelText('K'), { target: { value: 'hello' } })
    expect(onChange).toHaveBeenCalledWith('k', 'hello')
  })

  it('edits a textarea', () => {
    const { getByLabelText, onChange } = renderField({ key: 'k', label: 'K', type: 'textarea' })
    fireEvent.change(getByLabelText('K'), { target: { value: 'long text' } })
    expect(onChange).toHaveBeenCalledWith('k', 'long text')
  })

  it('coerces a number field to a number', () => {
    const { getByLabelText, onChange } = renderField({ key: 'n', label: 'N', type: 'number' })
    fireEvent.change(getByLabelText('N'), { target: { value: '42' } })
    // Not '42' — templates and dedupe both compare the raw value.
    expect(onChange).toHaveBeenCalledWith('n', 42)
  })

  it('reports a cleared number field as empty rather than NaN', () => {
    const { getByLabelText, onChange } = renderField({ key: 'n', label: 'N', type: 'number' }, 7)
    fireEvent.change(getByLabelText('N'), { target: { value: '' } })
    expect(onChange).toHaveBeenCalledWith('n', '')
  })

  it('renders a number field with an existing value without stringifying null', () => {
    const { getByLabelText } = renderField({ key: 'n', label: 'N', type: 'number' }, 7)
    expect(getByLabelText('N')).toHaveValue(7)
  })

  it('edits a select through the shared picker', () => {
    const { getByTestId, onChange } = renderField({
      key: 's',
      label: 'S',
      type: 'select',
      options: [
        { value: 'a', label: 'A' },
        { value: 'b', label: 'B' }
      ]
    })
    expect(getByTestId('select-picker')).toHaveAttribute('data-count', '2')
    fireEvent.click(getByTestId('select-picker'))
    expect(onChange).toHaveBeenCalledWith('s', 'b')
  })

  it('toggles a boolean and shows its label beside the checkbox', () => {
    const { getByLabelText, onChange } = renderField(
      { key: 'f', label: 'Force', type: 'boolean' },
      false
    )
    const box = getByLabelText('Force')
    expect(box).not.toBeChecked()
    fireEvent.click(box)
    expect(onChange).toHaveBeenCalledWith('f', true)
  })

  it('lets an optional project input be cleared but a required one not', () => {
    const opt = renderField({ key: 'p', label: 'P', type: 'project' })
    expect(opt.getByTestId('project-picker')).toHaveAttribute('data-allow-none', 'true')
    opt.unmount()

    const req = renderField({ key: 'p', label: 'P', type: 'project', required: true })
    expect(req.getByTestId('project-picker')).toHaveAttribute('data-allow-none', 'false')

    fireEvent.click(req.getByTestId('project-picker'))
    expect(req.onChange).toHaveBeenCalledWith('p', 'Vorn')
  })

  it('edits a branch input', () => {
    const { getByLabelText, onChange } = renderField({ key: 'b', label: 'Branch', type: 'branch' })
    fireEvent.change(getByLabelText('Branch'), { target: { value: 'main' } })
    expect(onChange).toHaveBeenCalledWith('b', 'main')
  })
})
