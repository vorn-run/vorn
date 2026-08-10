import { describe, it, expect } from 'vitest'
import { resolveWorkflowInputs } from '../packages/mcp/src/tools/workflows'
import type { WorkflowInputDef } from '../packages/shared/src/types'

const defs: WorkflowInputDef[] = [
  { key: 'topic', label: 'Topic', type: 'text', required: true },
  { key: 'hours', label: 'Window (hours)', type: 'number', defaultValue: '24' },
  { key: 'publish', label: 'Publish', type: 'boolean', defaultValue: 'false' },
  {
    key: 'tier',
    label: 'Tier',
    type: 'select',
    options: [
      { value: 'primary', label: 'Primary' },
      { value: 'secondary', label: 'Secondary' }
    ]
  }
]

describe('resolveWorkflowInputs', () => {
  it('applies declared defaults for values that were not supplied', () => {
    const { values, errors } = resolveWorkflowInputs(defs, { topic: 'agents' })
    expect(errors).toEqual([])
    expect(values).toEqual({ topic: 'agents', hours: 24, publish: false })
  })

  it('reports a missing required input', () => {
    const { errors } = resolveWorkflowInputs(defs, {})
    expect(errors).toEqual(['missing required input "topic" (Topic)'])
  })

  it('rejects an unknown key rather than silently dropping it', () => {
    const { errors } = resolveWorkflowInputs(defs, { topic: 'a', topci: 'typo' })
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('unknown input "topci"')
    expect(errors[0]).toContain('topic')
  })

  it('coerces numeric and boolean strings, since MCP args arrive as JSON scalars', () => {
    const { values, errors } = resolveWorkflowInputs(defs, {
      topic: 'a',
      hours: '48',
      publish: 'true'
    })
    expect(errors).toEqual([])
    expect(values.hours).toBe(48)
    expect(values.publish).toBe(true)
  })

  it('rejects a non-numeric value for a number input', () => {
    const { errors } = resolveWorkflowInputs(defs, { topic: 'a', hours: 'soon' })
    expect(errors).toEqual(['input "hours" must be a number, got "soon"'])
  })

  it('limits a select to its declared options', () => {
    const { errors } = resolveWorkflowInputs(defs, { topic: 'a', tier: 'tertiary' })
    expect(errors).toEqual(['input "tier" must be one of: primary, secondary'])
  })

  it('accepts a valid select value', () => {
    const { values, errors } = resolveWorkflowInputs(defs, { topic: 'a', tier: 'primary' })
    expect(errors).toEqual([])
    expect(values.tier).toBe('primary')
  })

  it('omits an optional input with no value and no default', () => {
    const { values } = resolveWorkflowInputs(defs, { topic: 'a' })
    expect(values).not.toHaveProperty('tier')
  })

  it('returns no values when nothing is declared', () => {
    const { values, errors } = resolveWorkflowInputs([], {})
    expect(values).toEqual({})
    expect(errors).toEqual([])
  })
})
