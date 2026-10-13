import { describe, it, expect } from 'vitest'
import { glowColorForAgent } from '../packages/web/src/renderer/lib/widget-glow'

describe('glowColorForAgent', () => {
  it('returns the AI-agent-specific color for each AI agent', () => {
    expect(glowColorForAgent('claude')).toContain('217, 119, 87')
    expect(glowColorForAgent('codex')).toContain('122, 157, 255')
    expect(glowColorForAgent('gemini')).toContain('49, 134, 255')
    expect(glowColorForAgent('copilot')).toContain('255, 255, 255')
    expect(glowColorForAgent('opencode')).toContain('255, 255, 255')
  })

  it('returns a neutral grey fallback for shell (no AI brand color)', () => {
    // shells shouldn't get a vivid brand glow; a muted grey is appropriate
    const shell = glowColorForAgent('shell')
    expect(shell).toContain('156, 163, 175')
    expect(shell).not.toEqual(glowColorForAgent('claude'))
  })
})
