import { describe, it, expect } from 'vitest'
import { inlineLogTail } from '../src/renderer/components/workflow-editor/node-visuals'

describe('inlineLogTail', () => {
  it('returns a short log unchanged', () => {
    expect(inlineLogTail('all of it')).toBe('all of it')
  })

  it('keeps the END of a long log, because that is where the answer is', () => {
    // The previous implementation sliced from 0, so a step that ended in its
    // verdict showed only the preamble.
    const logs = 'x'.repeat(20000) + '<<<VORN_OUTPUT>>>{"approved":true}'
    const shown = inlineLogTail(logs)
    expect(shown).toContain('<<<VORN_OUTPUT>>>{"approved":true}')
    expect(shown.endsWith('{"approved":true}')).toBe(true)
  })

  it('says how much it hid rather than trailing off', () => {
    const shown = inlineLogTail('y'.repeat(9000))
    expect(shown).toContain('1,000 earlier characters hidden')
    expect(shown).toContain('View full output')
  })

  it('does not annotate a log that fits', () => {
    expect(inlineLogTail('z'.repeat(8000))).not.toContain('hidden')
  })
})
