// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => null,
  useConnectionIconFor: () => undefined
}))

import { VariableAutocomplete } from '../src/renderer/components/workflow-editor/panels/VariableAutocomplete'
import type { StepVariableGroup } from '../src/renderer/lib/template-vars'

const triageGroup: StepVariableGroup = {
  nodeId: 'n1',
  label: 'Triage the issue',
  slug: 'triage',
  nodeType: 'launchAgent',
  runStatus: 'success',
  runCompletedAt: '2026-08-31T14:14:00Z',
  runOutputs: { severity: 'bug', summary: 'Crash on save' },
  keys: [
    { key: 'severity', label: 'severity', description: 'From the last run', value: '"bug"' },
    { key: 'output', label: 'Output', description: 'Primary output', value: '"done"' }
  ]
}

function open(value = '', groups = [triageGroup]) {
  const utils = render(
    <VariableAutocomplete value={value} onChange={vi.fn()} stepGroups={groups} contextVars={[]} />
  )
  const textarea = utils.container.querySelector('textarea')!
  fireEvent.change(textarea, { target: { value: value + '{{' } })
  return utils
}

describe('the picker carrying run data', () => {
  it('shows each entry beside its real last-run value', () => {
    const { getByText } = open()
    expect(getByText('"bug"')).toBeInTheDocument()
    expect(getByText('"done"')).toBeInTheDocument()
  })

  it('renders the group header as the step, with when it ran', () => {
    const { getByText, container } = open()
    expect(getByText('Triage the issue')).toBeInTheDocument()
    // The header carries the run's local time, not an uppercase category label.
    expect(container.textContent).toMatch(/\d{1,2}:\d{2}/)
  })
})

describe('the resolved preview under a field', () => {
  it('previews a template that resolves against the last run', () => {
    const { getByText } = render(
      <VariableAutocomplete
        value="Sev: {{steps.triage.severity}}"
        onChange={vi.fn()}
        stepGroups={[triageGroup]}
        contextVars={[]}
      />
    )
    expect(getByText('→ Sev: bug')).toBeInTheDocument()
  })

  it('flags a broken path and suggests the nearest real one', () => {
    const { getByText } = render(
      <VariableAutocomplete
        value="{{steps.triaje.severity}}"
        onChange={vi.fn()}
        stepGroups={[triageGroup]}
        contextVars={[]}
      />
    )
    expect(getByText(/steps\.triaje\.severity not found/)).toBeInTheDocument()
    expect(getByText(/did you mean steps\.triage\.severity\?/)).toBeInTheDocument()
  })
})
