// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { VariableAutocomplete } from '../src/renderer/components/workflow-editor/panels/VariableAutocomplete'
import type { TemplateVariable } from '../src/renderer/lib/template-vars'

describe('VariableAutocomplete', () => {
  it('lists context variables under a Context group when typing {{', () => {
    const contextVars: TemplateVariable[] = [
      { key: '{{context.cwd}}', label: 'cwd', category: 'context' },
      { key: '{{context.branch}}', label: 'branch', category: 'context' }
    ]
    const { container, getByText } = render(
      <VariableAutocomplete value="" onChange={vi.fn()} stepGroups={[]} contextVars={contextVars} />
    )
    const textarea = container.querySelector('textarea')!
    fireEvent.change(textarea, { target: { value: '{{' } })
    expect(getByText('Context')).toBeInTheDocument()
  })

  it('offers declared run inputs so they can be inserted from the picker', () => {
    const contextVars: TemplateVariable[] = [
      { key: '{{inputs.pr_number}}', label: 'PR number', category: 'inputs' }
    ]
    const onChange = vi.fn()
    const { container, getByText } = render(
      <VariableAutocomplete
        value=""
        onChange={onChange}
        stepGroups={[]}
        contextVars={contextVars}
      />
    )
    const textarea = container.querySelector('textarea')!
    fireEvent.change(textarea, { target: { value: '{{' } })

    expect(getByText('Run Inputs')).toBeInTheDocument()
    fireEvent.click(getByText('PR number'))
    // Inserting by hand is what produced unresolvable single-brace text before.
    expect(onChange).toHaveBeenCalledWith('{{inputs.pr_number}}')
  })

  it('offers connector item variables, which the picker used to drop', () => {
    const contextVars: TemplateVariable[] = [
      { key: '{{connectorItem.title}}', label: 'title', category: 'connectorItem' }
    ]
    const { container, getByText } = render(
      <VariableAutocomplete value="" onChange={vi.fn()} stepGroups={[]} contextVars={contextVars} />
    )
    const textarea = container.querySelector('textarea')!
    fireEvent.change(textarea, { target: { value: '{{' } })
    expect(getByText('Connector Item')).toBeInTheDocument()
  })
})
