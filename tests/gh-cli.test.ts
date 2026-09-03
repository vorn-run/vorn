import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/process-utils', () => ({
  getSafeEnv: () => ({ PATH: '/usr/bin' }),
  isAbsolutelyStrippedEnvName: (name: string) => name.startsWith('CLAUDE_CODE_'),
  getEnvPassthrough: () => new Set<string>(),
  SENSITIVE_ENV_PREFIXES: ['GITHUB_TOKEN', 'GH_TOKEN', 'ANTHROPIC_API']
}))

const importGhCli = async () => import('../packages/server/src/connectors/gh-cli')

const origPlatform = process.platform
function setPlatform(p: NodeJS.Platform): void {
  Object.defineProperty(process, 'platform', { value: p, configurable: true })
}

afterEach(() => {
  setPlatform(origPlatform)
})

describe('gh-cli — ghInstallHint()', () => {
  it('gives the Homebrew hint on darwin', async () => {
    setPlatform('darwin')
    const { ghInstallHint } = await importGhCli()
    expect(ghInstallHint()).toMatch(/brew install gh/)
  })

  it('gives the winget / cli.github.com hint on win32', async () => {
    setPlatform('win32')
    const { ghInstallHint } = await importGhCli()
    expect(ghInstallHint()).toMatch(/winget.*GitHub\.cli/)
    expect(ghInstallHint()).toMatch(/cli\.github\.com/)
  })

  it('gives a generic hint on linux', async () => {
    setPlatform('linux')
    const { ghInstallHint } = await importGhCli()
    expect(ghInstallHint()).toMatch(/apt install gh/)
  })
})

describe('gh-cli — GhNotFoundError', () => {
  it('has a GH_NOT_FOUND code and embeds the install hint in its message', async () => {
    setPlatform('darwin')
    const { GhNotFoundError } = await importGhCli()
    const err = new GhNotFoundError()
    expect(err.code).toBe('GH_NOT_FOUND')
    expect(err.name).toBe('GhNotFoundError')
    expect(err.message).toMatch(/GitHub CLI .* not found/)
    expect(err.message).toMatch(/brew install gh/)
  })
})

describe('gh-cli — getGhEnv()', () => {
  const origGhToken = process.env.GH_TOKEN
  const origGithubToken = process.env.GITHUB_TOKEN

  beforeEach(() => {
    delete process.env.GH_TOKEN
    delete process.env.GITHUB_TOKEN
  })

  afterEach(() => {
    if (origGhToken !== undefined) process.env.GH_TOKEN = origGhToken
    else delete process.env.GH_TOKEN
    if (origGithubToken !== undefined) process.env.GITHUB_TOKEN = origGithubToken
    else delete process.env.GITHUB_TOKEN
  })

  it('starts from the safe env (login-shell PATH)', async () => {
    const { getGhEnv } = await importGhCli()
    const env = getGhEnv()
    expect(env.PATH).toBe('/usr/bin')
  })

  it('preserves GH_TOKEN and GITHUB_TOKEN from process.env', async () => {
    process.env.GH_TOKEN = 'gh-secret'
    process.env.GITHUB_TOKEN = 'gha-secret'
    const { getGhEnv } = await importGhCli()
    const env = getGhEnv()
    expect(env.GH_TOKEN).toBe('gh-secret')
    expect(env.GITHUB_TOKEN).toBe('gha-secret')
  })

  it('omits GH_TOKEN / GITHUB_TOKEN entirely when they are not set', async () => {
    const { getGhEnv } = await importGhCli()
    const env = getGhEnv()
    expect(env.GH_TOKEN).toBeUndefined()
    expect(env.GITHUB_TOKEN).toBeUndefined()
  })
})
