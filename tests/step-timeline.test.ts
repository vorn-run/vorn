import { describe, it, expect } from 'vitest'
import { stepTimeline } from '../src/renderer/components/workflow-editor/node-visuals'

/**
 * The timeline is the one place a stalled step accounts for itself, so the
 * order it renders in — setup, agent, outcome — is the behaviour under test.
 */
describe('stepTimeline', () => {
  const launch = '[+0.0s] Launching claude in /repo with a 12-line prompt'
  const firstOutput = '[+1.4s] First output from the agent (128 bytes)'
  const exited = '[+9.2s] Agent exited (code 0)'

  it('puts the agent between the engine notes that bracket it', () => {
    const entries = stepTimeline('hello from the agent', [launch, firstOutput, exited].join('\n'))

    expect(entries.map((e) => e.kind)).toEqual(['engine', 'engine', 'agent', 'engine'])
    expect(entries[1].text).toBe(firstOutput)
    expect(entries[2].text).toBe('hello from the agent')
    expect(entries[3].text).toBe(exited)
  })

  it('keeps engine notes alone when the agent never wrote anything', () => {
    const entries = stepTimeline(undefined, [launch, exited].join('\n'))

    expect(entries).toEqual([
      { kind: 'engine', text: launch },
      { kind: 'engine', text: exited }
    ])
  })

  it('treats whitespace-only logs as no output', () => {
    const entries = stepTimeline('   \n  ', launch)

    expect(entries.every((e) => e.kind === 'engine')).toBe(true)
  })

  it('appends the agent after the notes when no first-output seam was recorded', () => {
    // A step can end before the engine ever notes a first byte — the agent's
    // words still belong after the setup that produced them.
    const entries = stepTimeline('late output', launch)

    expect(entries.map((e) => e.kind)).toEqual(['engine', 'agent'])
  })

  it('renders the agent alone when there are no engine notes', () => {
    expect(stepTimeline('just output', undefined)).toEqual([{ kind: 'agent', text: 'just output' }])
  })

  it('is empty when the step produced nothing at all', () => {
    expect(stepTimeline(undefined, undefined)).toEqual([])
    expect(stepTimeline('', '')).toEqual([])
  })
})
