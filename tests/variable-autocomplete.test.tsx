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
})
