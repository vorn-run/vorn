import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from 'vitest'

vi.mock('node:child_process', async () => {
  const actual = await vi.importActual<typeof import('node:child_process')>('node:child_process')
  return {
    ...actual,
    execSync: vi.fn(() => {
      throw new Error('mock shell env')
    }),
    execFileSync: vi.fn(() => {
      throw new Error('mock shell env')
    })
  }
})
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn()
}))

let shellEscape: typeof import('../packages/server/src/process-utils').shellEscape
let getSafeEnv: typeof import('../packages/server/src/process-utils').getSafeEnv

const TEST_ENV = {
  HOME: '/home/user',
  PATH: '/usr/bin',
  SHELL: '/bin/zsh',
  GITHUB_TOKEN: 'ghp_secret123',
  GH_TOKEN: 'ghp_secret456',
  AWS_SECRET_ACCESS_KEY: 'aws-secret',
  AWS_SESSION_TOKEN: 'aws-session',
  OPENAI_API_KEY: 'sk-openai',
  ANTHROPIC_API_KEY: 'sk-ant',
  GOOGLE_API_KEY: 'google-key',
  STRIPE_SECRET_KEY: 'sk_stripe',
  DATABASE_URL: 'postgres://localhost/db',
  DB_PASSWORD: 'dbpass',
  SECRET_KEY: 'mysecret',
  PRIVATE_KEY_PATH: '/path/to/key',
  NPM_TOKEN: 'npm-token',
  NODE_AUTH_TOKEN: 'node-auth',
  EDITOR: 'vim',
  TERM: 'xterm-256color'
}

describe('shellEscape', () => {
  beforeAll(async () => {
    vi.resetModules()
    ;({ shellEscape } = await import('../packages/server/src/process-utils'))
  })

  it('returns safe strings unquoted', () => {
    expect(shellEscape('hello')).toBe('hello')
  })

  it('escapes single quotes', () => {
    expect(shellEscape("it's")).toBe("'it'\\''s'")
  })

  it('handles empty string', () => {
    expect(shellEscape('')).toBe("''")
  })

  it('wraps string with spaces', () => {
    expect(shellEscape('a b c')).toBe("'a b c'")
  })

  it('safely wraps backticks', () => {
    expect(shellEscape('`whoami`')).toBe("'`whoami`'")
  })

  it('safely wraps $() subshell', () => {
    expect(shellEscape('$(rm -rf /)')).toBe("'$(rm -rf /)'")
  })

  it('safely wraps double quotes', () => {
    expect(shellEscape('say "hi"')).toBe('\'say "hi"\'')
  })

  it('handles multiple single quotes', () => {
    expect(shellEscape("it's a 'test'")).toBe("'it'\\''s a '\\''test'\\'''")
  })
})

describe('getSafeEnv', () => {
  const originalEnv = process.env

  beforeEach(async () => {
    vi.resetModules()
    process.env = { ...TEST_ENV }
    ;({ getSafeEnv } = await import('../packages/server/src/process-utils'))
  })

  afterEach(() => {
    process.env = originalEnv
  })

  it('keeps safe environment variables', () => {
    const env = getSafeEnv()
    expect(env.HOME).toBe('/home/user')
    expect(env.PATH).toBe('/usr/bin')
    expect(env.SHELL).toBe('/bin/zsh')
    expect(env.EDITOR).toBe('vim')
    expect(env.TERM).toBe('xterm-256color')
  })

  it('filters out GITHUB_TOKEN', () => {
    expect(getSafeEnv().GITHUB_TOKEN).toBeUndefined()
  })

  it('filters out GH_TOKEN', () => {
    expect(getSafeEnv().GH_TOKEN).toBeUndefined()
  })

  it('filters out AWS_SECRET_ACCESS_KEY', () => {
    expect(getSafeEnv().AWS_SECRET_ACCESS_KEY).toBeUndefined()
  })

  it('filters out AWS_SESSION_TOKEN', () => {
    expect(getSafeEnv().AWS_SESSION_TOKEN).toBeUndefined()
  })

  it('filters out OPENAI_API_KEY', () => {
    expect(getSafeEnv().OPENAI_API_KEY).toBeUndefined()
  })

  it('filters out ANTHROPIC_API_KEY', () => {
    expect(getSafeEnv().ANTHROPIC_API_KEY).toBeUndefined()
  })

  it('filters out GOOGLE_API_KEY', () => {
    expect(getSafeEnv().GOOGLE_API_KEY).toBeUndefined()
  })

  it('filters out STRIPE_ prefixed vars', () => {
    expect(getSafeEnv().STRIPE_SECRET_KEY).toBeUndefined()
  })

  it('filters out DATABASE_URL', () => {
    expect(getSafeEnv().DATABASE_URL).toBeUndefined()
  })

  it('filters out DB_PASSWORD', () => {
    expect(getSafeEnv().DB_PASSWORD).toBeUndefined()
  })

  it('filters out SECRET_ prefixed vars', () => {
    expect(getSafeEnv().SECRET_KEY).toBeUndefined()
  })

  it('filters out PRIVATE_KEY prefixed vars', () => {
    expect(getSafeEnv().PRIVATE_KEY_PATH).toBeUndefined()
  })

  it('filters out NPM_TOKEN', () => {
    expect(getSafeEnv().NPM_TOKEN).toBeUndefined()
  })

  it('filters out NODE_AUTH_TOKEN', () => {
    expect(getSafeEnv().NODE_AUTH_TOKEN).toBeUndefined()
  })
})
