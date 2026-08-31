import { describe, it, expect } from 'vitest'
import {
  buildStepGroups,
  formatRunValue,
  previewStepTokens,
  suggestNearestPath,
  type LastRunData
} from '../src/renderer/lib/template-vars'
import type { WorkflowNode } from '../packages/shared/src/types'

const agent = (id: string, slug: string): WorkflowNode =>
  ({
    id,
    type: 'launchAgent',
    label: slug,
    slug,
    position: { x: 0, y: 0 },
    config: { agentType: 'claude', projectName: '', projectPath: '', headless: true }
  }) as unknown as WorkflowNode

const lastRun: LastRunData = {
  outputs: {
    triage: { output: 'done', status: 'success', error: '', severity: 'bug' }
  },
  states: { n1: { status: 'success', completedAt: '2026-08-31T14:14:00Z' } }
}

describe('step groups carrying the last run', () => {
  it('attaches a rendered value and the run identity to each group', () => {
    const groups = buildStepGroups([agent('n1', 'triage')], undefined, lastRun)
    expect(groups[0].runStatus).toBe('success')
    expect(groups[0].runCompletedAt).toBe('2026-08-31T14:14:00Z')
    expect(groups[0].keys.find((k) => k.key === 'output')?.value).toBe('"done"')
  })

  it('leaves values off when the workflow has never run', () => {
    const groups = buildStepGroups([agent('n1', 'triage')])
    expect(groups[0].runStatus).toBeUndefined()
    expect(groups[0].keys.every((k) => k.value === undefined)).toBe(true)
  })
})

describe('formatRunValue', () => {
  it('quotes strings, flattens whitespace, and clips long values', () => {
    expect(formatRunValue('a\n b')).toBe('"a b"')
    expect(formatRunValue(42)).toBe('42')
    expect(formatRunValue('x'.repeat(100))).toContain('…')
  })
})

describe('previewing steps tokens', () => {
  const groups = buildStepGroups([agent('n1', 'triage')], undefined, lastRun)

  it('resolves a known path against the last run', () => {
    const preview = previewStepTokens('Sev: {{steps.triage.severity}}', groups)
    expect(preview.resolved).toBe('Sev: bug')
    expect(preview.broken).toBeUndefined()
  })

  it('flags a path that resolves nowhere and suggests the nearest real one', () => {
    const preview = previewStepTokens('{{steps.triaje.severity}}', groups)
    expect(preview.broken?.token).toBe('steps.triaje.severity')
    expect(preview.broken?.suggestion).toBe('steps.triage.severity')
  })

  it('stays quiet with no steps tokens or no run data', () => {
    expect(previewStepTokens('plain text', groups)).toEqual({})
    const dry = buildStepGroups([agent('n1', 'triage')])
    expect(previewStepTokens('{{steps.triage.output}}', dry)).toEqual({})
  })
})

describe('suggestNearestPath', () => {
  it('offers nothing when everything is far away', () => {
    expect(suggestNearestPath('steps.zzzzzzzzzzzz.qqq', ['steps.triage.output'])).toBeUndefined()
  })
})
