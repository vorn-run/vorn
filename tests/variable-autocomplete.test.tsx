// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { VariableAutocomplete } from '../src/renderer/components/workflow-editor/panels/VariableAutocomplete'
import type { TemplateVariable } from '@vornrun/shared/template-vars'

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

describe('what a gate offers the steps after it', () => {
  it('offers its page comments beside its text and notes, described as they read', async () => {
    const { buildStepGroups } = await import('@vornrun/shared/template-vars')
    const gate = {
      id: 'g',
      type: 'approval',
      slug: 'my_review',
      label: 'Review the draft',
      config: {},
      position: { x: 0, y: 0 }
    } as unknown as Parameters<typeof buildStepGroups>[0][number]
    const [group] = buildStepGroups([gate])
    const described = Object.fromEntries(group.keys.map((k) => [k.key, k.description]))
    expect(described.comments).toBe(
      'Anchored comments from the latest round, as JSON: quote, comment, anchored'
    )
    expect(described.feedback).toBe('The general note from the latest round')
    expect(described.feedbackAll).toBe('Every note, one line per round')
    expect(described.text).toBeTruthy()
  })
})
