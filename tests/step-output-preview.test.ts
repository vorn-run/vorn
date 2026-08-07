import { describe, it, expect } from 'vitest'
import { stepOutputPreview } from '../src/renderer/components/workflow-editor/node-visuals'
import type { NodeExecutionState } from '../src/shared/types'

function state(overrides: Partial<NodeExecutionState> = {}): NodeExecutionState {
  return { nodeId: 'n1', status: 'success', ...overrides } as NodeExecutionState
}

describe('stepOutputPreview', () => {
  it('returns the first non-blank line of the log', () => {
    expect(stepOutputPreview(state({ logs: 'installing deps\nrunning tests\nall green' }))).toBe(
      'installing deps'
    )
  })

  it('skips leading blank lines and trims the one it finds', () => {
    expect(stepOutputPreview(state({ logs: '\n\n   \n  hello  \nworld' }))).toBe('hello')
  })

  it('falls back to the error when a step produced no logs', () => {
    expect(stepOutputPreview(state({ error: '  boom  ' }))).toBe('boom')
  })

  it('returns nothing for an empty or whitespace-only log', () => {
    expect(stepOutputPreview(state())).toBeUndefined()
    expect(stepOutputPreview(state({ logs: '   \n\n  ' }))).toBeUndefined()
  })

  it('clips a log that has no newline at all', () => {
    const preview = stepOutputPreview(state({ logs: 'x'.repeat(10_000) }))
    expect(preview).toHaveLength(300)
  })

  it('gives up rather than scanning a huge log for its first visible line', () => {
    // A log that is blank far past the scan limit costs a bounded amount of
    // work: an agent step re-renders on every streamed chunk, so this must not
    // grow with the size of the log.
    const logs = '\n'.repeat(50_000) + 'finally something'
    expect(stepOutputPreview(state({ logs }))).toBeUndefined()
  })

  it('never slices more than a preview line, even from one enormous line', () => {
    // The blank run is a single "line" longer than the whole scan budget, so
    // an implementation that sliced to the newline first would copy all of it.
    const logs = ' '.repeat(50_000) + '\nfinally something'
    expect(stepOutputPreview(state({ logs }))).toBeUndefined()
  })
})
