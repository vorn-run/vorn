import { describe, it, expect } from 'vitest'
import {
  filterEnv,
  normalizePassthrough,
  SENSITIVE_ENV_PREFIXES
} from '../packages/server/src/process-utils'

const source = {
  PATH: '/usr/bin',
  HOME: '/Users/someone',
  GITHUB_TOKEN: 'ghp_real',
  ANTHROPIC_API_KEY: 'sk-ant-real',
  ANTHROPIC_BASE_URL: 'http://localhost:3000',
  CLAUDECODE: '1',
  UNSET: undefined
}

describe('filterEnv', () => {
  it('strips sensitive variables by default', () => {
    const env = filterEnv(source, new Set())
    expect(env.GITHUB_TOKEN).toBeUndefined()
    expect(env.ANTHROPIC_API_KEY).toBeUndefined()
  })

  it('keeps ordinary variables', () => {
    const env = filterEnv(source, new Set())
    expect(env.PATH).toBe('/usr/bin')
    expect(env.HOME).toBe('/Users/someone')
  })

  it('drops undefined values instead of forwarding "undefined"', () => {
    const env = filterEnv(source, new Set())
    expect(env).not.toHaveProperty('UNSET')
  })

  it('forwards a variable the user named, overriding the prefix rule', () => {
    const env = filterEnv(source, new Set(['ANTHROPIC_API_KEY']))
    expect(env.ANTHROPIC_API_KEY).toBe('sk-ant-real')
    // Naming one does not open the rest.
    expect(env.GITHUB_TOKEN).toBeUndefined()
  })

  it('strips STRIP_ENV_KEYS whatever their casing', () => {
    // Windows env names are case-insensitive and Node preserves the casing it
    // enumerated, so a literal match would let this through.
    const env = filterEnv({ claudecode: '1', ClaudeCode: '1' }, new Set())
    expect(env).not.toHaveProperty('claudecode')
    expect(env).not.toHaveProperty('ClaudeCode')
  })

  it('never forwards STRIP_ENV_KEYS, even when named', () => {
    // CLAUDECODE is stripped so nested agent CLIs still launch; forwarding it
    // breaks the session rather than protecting anything.
    const env = filterEnv(source, new Set(['CLAUDECODE']))
    expect(env).not.toHaveProperty('CLAUDECODE')
  })

  it('already allowed ANTHROPIC_BASE_URL, which is why the key alone was the gap', () => {
    const env = filterEnv(source, new Set())
    expect(env.ANTHROPIC_BASE_URL).toBe('http://localhost:3000')
  })

  it('matches prefixes case-insensitively', () => {
    const env = filterEnv({ github_token: 'x' }, new Set())
    expect(env).not.toHaveProperty('github_token')
  })

  it('still lists the prefixes it protects', () => {
    expect(SENSITIVE_ENV_PREFIXES).toContain('ANTHROPIC_API')
    expect(SENSITIVE_ENV_PREFIXES).toContain('GITHUB_TOKEN')
  })
})

describe('normalizePassthrough', () => {
  it('uppercases and trims, so config casing does not matter', () => {
    expect(normalizePassthrough([' anthropic_api_key '])).toEqual(new Set(['ANTHROPIC_API_KEY']))
  })

  it('treats undefined as empty', () => {
    expect(normalizePassthrough(undefined)).toEqual(new Set())
  })

  it('drops blank entries', () => {
    expect(normalizePassthrough(['', '   ', 'X'])).toEqual(new Set(['X']))
  })

  // The value comes from JSON.parse of a hand-editable file, so the declared
  // string[] is a hope. Throwing here would take the server down at startup.
  it('survives a non-array value', () => {
    expect(normalizePassthrough('ANTHROPIC_API_KEY')).toEqual(new Set())
    expect(normalizePassthrough(42)).toEqual(new Set())
    expect(normalizePassthrough(null)).toEqual(new Set())
  })

  it('skips non-string entries rather than throwing', () => {
    expect(normalizePassthrough(['GOOD', 42, null, { a: 1 }])).toEqual(new Set(['GOOD']))
  })
})
