import { describe, it, expect } from 'vitest'
import { displayNameFromPrompt } from '@vornrun/shared/string-utils'

/**
 * The name a prompt produces, refusing rather than assuming there is one.
 *
 * `displayNameFromPrompt` returns undefined for a prompt with nothing in it, and
 * every test here is about a prompt that has something.
 */
function nameOf(prompt: string): string {
  const name = displayNameFromPrompt(prompt)
  if (name === undefined) throw new Error(`no display name for: ${prompt}`)
  return name
}

describe('displayNameFromPrompt', () => {
  it('returns short prompt as-is', () => {
    expect(displayNameFromPrompt('Fix the login bug')).toBe('Fix the login bug')
  })

  it('truncates at word boundary with ellipsis', () => {
    const long =
      'Refactor the authentication module to use JWT tokens instead of session cookies for better scalability'
    const result = nameOf(long)
    expect(result.length).toBeLessThanOrEqual(61) // 60 + ellipsis char
    expect(result).toMatch(/\u2026$/)
    expect(result).not.toMatch(/\s\u2026$/) // no trailing space before ellipsis
  })

  it('strips leading whitespace and newlines', () => {
    expect(displayNameFromPrompt('\n\n  Fix the bug')).toBe('Fix the bug')
  })

  it('collapses internal whitespace', () => {
    expect(displayNameFromPrompt('Fix   the\n\nbug')).toBe('Fix the bug')
  })

  it('handles prompt that is exactly maxLen', () => {
    const exact = 'a'.repeat(60)
    expect(displayNameFromPrompt(exact)).toBe(exact)
  })

  it('handles single long word exceeding maxLen', () => {
    const word = 'a'.repeat(80)
    const result = nameOf(word)
    expect(result).toBe('a'.repeat(60) + '\u2026')
  })

  it('respects custom maxLen', () => {
    const result = displayNameFromPrompt('Hello beautiful world out there', 15)
    if (result === undefined) throw new Error('expected a name for a non-empty prompt')
    expect(result.length).toBeLessThanOrEqual(16)
    expect(result).toMatch(/\u2026$/)
  })

  it('returns undefined for whitespace-only input', () => {
    expect(displayNameFromPrompt('   \n\t  ')).toBeUndefined()
  })
})
