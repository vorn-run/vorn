import { describe, it, expect } from 'vitest'
import {
  isAiAgent,
  supportsExactSessionResume,
  supportsSessionIdPinning,
  getSessionIdPinningFlag,
  getRecentSessionActivityLabel,
  type AgentType,
  type AiAgentType
} from '../packages/shared/src/types'

describe('shell as an AgentType', () => {
  const AI_AGENTS: AiAgentType[] = ['claude', 'copilot', 'codex', 'gemini']

  describe('isAiAgent type guard', () => {
    it('returns true for every AI agent', () => {
      for (const a of AI_AGENTS) {
        expect(isAiAgent(a)).toBe(true)
      }
    })

    it('returns false for shell', () => {
      expect(isAiAgent('shell')).toBe(false)
    })

    it('returns false for undefined', () => {
      expect(isAiAgent(undefined)).toBe(false)
    })

    it('narrows the type so callers can safely index AI-agent-only maps', () => {
      const candidate: AgentType = 'claude'
      if (isAiAgent(candidate)) {
        // This line would not compile if narrowing were broken.
        const _check: AiAgentType = candidate
        expect(_check).toBe('claude')
      } else {
        throw new Error('claude should be an AI agent')
      }
    })
  })

  describe('supportsExactSessionResume', () => {
    it('returns false for shell (no resume concept for plain PTYs)', () => {
      expect(supportsExactSessionResume('shell')).toBe(false)
    })

    it('returns false for gemini (pre-existing behavior preserved)', () => {
      expect(supportsExactSessionResume('gemini')).toBe(false)
    })

    it('returns true for every other AI agent', () => {
      for (const a of ['claude', 'copilot', 'codex'] as AiAgentType[]) {
        expect(supportsExactSessionResume(a)).toBe(true)
      }
    })
  })

  describe('supportsSessionIdPinning', () => {
    it('returns false for shell', () => {
      expect(supportsSessionIdPinning('shell')).toBe(false)
    })

    it('returns true only for claude and copilot (unchanged)', () => {
      expect(supportsSessionIdPinning('claude')).toBe(true)
      expect(supportsSessionIdPinning('copilot')).toBe(true)
      expect(supportsSessionIdPinning('codex')).toBe(false)
      expect(supportsSessionIdPinning('gemini')).toBe(false)
    })
  })

  describe('getSessionIdPinningFlag', () => {
    it('throws for shell — shells have no pinning flag', () => {
      expect(() => getSessionIdPinningFlag('shell')).toThrow(/does not support session ID pinning/)
    })
  })

  describe('getRecentSessionActivityLabel', () => {
    it('returns "line" for shell', () => {
      expect(getRecentSessionActivityLabel('shell')).toBe('line')
    })

    it('returns the existing labels for AI agents', () => {
      expect(getRecentSessionActivityLabel('claude')).toBe('entry')
      expect(getRecentSessionActivityLabel('codex')).toBe('entry')
      expect(getRecentSessionActivityLabel('copilot')).toBe('turn')
      expect(getRecentSessionActivityLabel('gemini')).toBe('prompt')
    })
  })
})
