// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { DynamicField } from '../src/renderer/components/settings/DynamicField'
import type { ConnectorConfigField } from '../src/shared/types'

const field = (overrides: Partial<ConnectorConfigField> = {}): ConnectorConfigField =>
  ({ key: 'token', label: 'API token', ...overrides }) as ConnectorConfigField

function setup(f: ConnectorConfigField, value = '') {
  const onChange = vi.fn()
  return { ...render(<DynamicField field={f} value={value} onChange={onChange} />), onChange }
}

describe('DynamicField', () => {
  it('labels the field and reports what was typed', () => {
    const { getByLabelText, onChange } = setup(field())
    fireEvent.change(getByLabelText(/API token/), { target: { value: 'abc' } })
    expect(onChange).toHaveBeenCalledWith('abc')
  })

  it('masks a password and says it will be encrypted', () => {
    // Someone pasting a secret into a form should be able to see that it is
    // not going to sit in a config file in the clear.
    const { getByLabelText, getByText } = setup(field({ type: 'password' }))
    expect(getByLabelText(/API token/)).toHaveAttribute('type', 'password')
    expect(getByText(/encrypted/)).toBeInTheDocument()
  })

  it('marks a required field, so a form is not failed on submit for a surprise', () => {
    const { getByText } = setup(field({ required: true }))
    expect(getByText('*')).toBeInTheDocument()
  })

  it('offers the declared choices, with a way back to none', () => {
    const { getByLabelText, onChange } = setup(
      field({
        type: 'select',
        options: [
          { value: 'a', label: 'Alpha' },
          { value: 'b', label: 'Beta' }
        ]
      })
    )
    const select = getByLabelText(/API token/) as HTMLSelectElement
    expect([...select.options].map((o) => o.textContent)).toEqual(['—', 'Alpha', 'Beta'])
    fireEvent.change(select, { target: { value: 'b' } })
    expect(onChange).toHaveBeenCalledWith('b')
  })

  it('gives a query somewhere to breathe rather than a single line', () => {
    const { getByLabelText, onChange } = setup(field({ type: 'textarea', label: 'Query' }))
    const box = getByLabelText(/Query/)
    expect(box.tagName).toBe('TEXTAREA')
    fireEvent.change(box, { target: { value: 'Alerts | take 1' } })
    expect(onChange).toHaveBeenCalledWith('Alerts | take 1')
  })

  it('shows the description the connector wrote, and nothing when it wrote none', () => {
    expect(
      setup(field({ description: 'Cluster URL' })).getByText('Cluster URL')
    ).toBeInTheDocument()
    expect(setup(field()).container.querySelector('p')).toBeNull()
  })

  it('shows a placeholder as a hint rather than a value', () => {
    const { getByPlaceholderText } = setup(field({ placeholder: 'contoso' }))
    expect(getByPlaceholderText('contoso')).toHaveValue('')
  })
})
