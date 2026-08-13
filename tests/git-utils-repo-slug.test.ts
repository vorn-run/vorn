import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn()
}))

vi.mock('node:fs', () => ({
  default: { mkdirSync: vi.fn(), existsSync: vi.fn(() => true) }
}))

vi.mock('node:crypto', () => ({
  default: { randomUUID: vi.fn(() => 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee') }
}))

import { execFileSync } from 'node:child_process'
import { parseGitHubRemote, detectRepoSlug } from '../packages/server/src/git-utils'

const exec = vi.mocked(execFileSync)

beforeEach(() => {
  exec.mockReset()
})

/**
 * Repo detection reads git rather than `gh repo view`, so it keeps working on
 * a machine with no GitHub CLI — the connector that needs `gh` is packaged and
 * separate, and this is the one thing Vorn itself wanted to know.
 */
describe('parseGitHubRemote', () => {
  it('reads the scp-like ssh form git writes by default', () => {
    expect(parseGitHubRemote('git@github.com:vorn-run/vorn.git')).toEqual({
      owner: 'vorn-run',
      repo: 'vorn'
    })
  })

  it('reads the https form', () => {
    expect(parseGitHubRemote('https://github.com/vorn-run/connectors.git')).toEqual({
      owner: 'vorn-run',
      repo: 'connectors'
    })
  })

  it('reads the ssh:// url form', () => {
    expect(parseGitHubRemote('ssh://git@github.com/vorn-run/vorn')).toEqual({
      owner: 'vorn-run',
      repo: 'vorn'
    })
  })

  it('tolerates a missing .git, a trailing slash and surrounding whitespace', () => {
    expect(parseGitHubRemote('  https://github.com/a/b/  ')).toEqual({ owner: 'a', repo: 'b' })
  })

  it('keeps a dot inside the repo name rather than treating it as the .git suffix', () => {
    expect(parseGitHubRemote('git@github.com:owner/my.repo.git')).toEqual({
      owner: 'owner',
      repo: 'my.repo'
    })
  })

  // A GitLab remote parses into the same shape. Handing that slug to a GitHub
  // connection would produce a connection that looks configured and returns
  // nothing, which is worse than refusing to guess.
  it('refuses a remote that is not github.com', () => {
    expect(parseGitHubRemote('git@gitlab.com:vorn-run/vorn.git')).toBeNull()
    expect(parseGitHubRemote('https://bitbucket.org/a/b.git')).toBeNull()
  })

  it('refuses a url that is not a repo root', () => {
    expect(parseGitHubRemote('https://github.com/vorn-run/vorn/tree/main')).toBeNull()
  })

  it('refuses empty and malformed input', () => {
    expect(parseGitHubRemote('')).toBeNull()
    expect(parseGitHubRemote('   ')).toBeNull()
    expect(parseGitHubRemote('github.com')).toBeNull()
    expect(parseGitHubRemote('https://github.com/onlyowner')).toBeNull()
  })
})

describe('detectRepoSlug', () => {
  it('asks git for the origin url and parses it', () => {
    // getSafeEnv() shells out for a login-shell PATH first, so the git call is
    // found by its arguments rather than by being first.
    exec.mockReturnValue('git@github.com:vorn-run/vorn.git\n' as unknown as string)
    expect(detectRepoSlug('/repo')).toEqual({ owner: 'vorn-run', repo: 'vorn' })
    const gitCall = exec.mock.calls.find(
      (call) => Array.isArray(call[1]) && call[1][0] === 'remote'
    )
    expect(gitCall?.[1]).toEqual(['remote', 'get-url', 'origin'])
  })

  // No repo, no origin, or no git at all. All mean "cannot tell", which the
  // caller turns into a manual owner/repo entry rather than an error.
  it('reports null when git fails rather than throwing at the caller', () => {
    exec.mockImplementation(() => {
      throw new Error('not a git repository')
    })
    expect(detectRepoSlug('/nowhere')).toBeNull()
  })

  it('reports null when origin points somewhere that is not GitHub', () => {
    exec.mockReturnValue('git@gitlab.com:a/b.git\n' as unknown as string)
    expect(detectRepoSlug('/repo')).toBeNull()
  })
})
