import { describe, it, expect, vi, afterEach } from 'vitest'
import type { SdkConnectorAuth } from '../packages/shared/src/types'
import {
  borrowedEnv,
  borrowedSecrets,
  identityFrom,
  installHintFor,
  probeAuth,
  signInCommand
} from '../packages/server/src/connectors/auth-rung'

const CLI: SdkConnectorAuth = {
  rung: 'cli',
  probe: { command: 'glab', args: ['auth', 'status'] },
  borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
}

/** A tool that is on PATH and answers however the test says. */
const found = (stdout = '', stderr = '') => ({
  resolve: () => '/usr/bin/glab',
  run: vi.fn().mockResolvedValue({ stdout, stderr })
})

afterEach(() => {
  vi.unstubAllEnvs()
})

describe('asking a borrowed tool who you are', () => {
  it('reports the account the tool names', async () => {
    const deps = found('', 'Logged in to gitlab.com account javier (keyring)')
    expect(await probeAuth(CLI, deps)).toEqual({ ok: true, identity: 'javier' })
  })

  it('runs the probe the connector declared, with its own borrowed variables', async () => {
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const deps = found('Logged in as javier')
    await probeAuth(CLI, deps)
    const [file, args, options] = deps.run.mock.calls[0]
    expect(file).toBe('/usr/bin/glab')
    expect(args).toEqual(['auth', 'status'])
    expect(options.env.GITLAB_TOKEN).toBe('from-the-shell')
  })

  it('names the sign-in to run when the tool says no', async () => {
    const deps = { resolve: () => '/usr/bin/glab', run: vi.fn().mockRejectedValue(new Error('1')) }
    const report = await probeAuth(CLI, deps)
    expect(report.ok).toBe(false)
    expect(report.message).toContain('glab auth login')
  })

  it('offers a way to get the tool when it is the tool that is missing', async () => {
    const report = await probeAuth(CLI, { resolve: () => null, run: vi.fn() })
    expect(report.ok).toBe(false)
    expect(report.message).toContain('not installed')
    expect(report.installHint).toBeTruthy()
  })

  it('answers a rung this probe cannot speak to with nothing rather than a no', async () => {
    expect(await probeAuth({ rung: 'key', keys: ['token'] })).toEqual({ ok: null })
    expect(await probeAuth(undefined)).toEqual({ ok: null })
  })

  it('calls a connector that needs no sign-in ready', async () => {
    const report = await probeAuth({ rung: 'none' })
    expect(report.ok).toBe(true)
    expect(report.message).toContain('Nothing to sign in to')
  })
})

describe('what a borrowed tool hands over', () => {
  it('passes through a variable the shell already set, without running anything', async () => {
    vi.stubEnv('GITLAB_TOKEN', 'already-here')
    const deps = found('printed-token')
    expect(await borrowedSecrets(CLI, deps)).toEqual({ GITLAB_TOKEN: 'already-here' })
    expect(deps.run).not.toHaveBeenCalled()
  })

  it('asks the tool for a token when nothing set it', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    const deps = found('printed-token\n')
    expect(await borrowedSecrets(CLI, deps)).toEqual({ GITLAB_TOKEN: 'printed-token' })
    expect(deps.run.mock.calls[0][1]).toEqual(['auth', 'token'])
  })

  it('borrows nothing for a rung that does not borrow', async () => {
    const deps = found('printed-token')
    expect(await borrowedSecrets({ rung: 'key', keys: ['token'] }, deps)).toEqual({})
    expect(deps.run).not.toHaveBeenCalled()
  })

  it('leaves the child to fail on its own when the tool will not hand a token over', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    const deps = { resolve: () => '/usr/bin/glab', run: vi.fn().mockRejectedValue(new Error('no')) }
    expect(await borrowedSecrets(CLI, deps)).toEqual({})
  })

  it('keeps a declared variable out of the environment when nothing set it', () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    expect(borrowedEnv(CLI).GITLAB_TOKEN).toBeUndefined()
  })
})

describe('reading an identity out of what a tool printed', () => {
  it('prefers the account a tool names over its first line', () => {
    expect(identityFrom('github.com\n  ✓ Logged in to github.com account javier (keyring)')).toBe(
      'javier'
    )
  })

  it('reads the other phrasing tools use', () => {
    expect(identityFrom('Logged in to gitlab.com as javier.')).toBe('javier')
  })

  it('falls back to the first line it was given', () => {
    expect(identityFrom('\n\nSubscription: Pay-As-You-Go\n')).toBe('Subscription: Pay-As-You-Go')
  })

  it('says nothing about an empty answer', () => {
    expect(identityFrom('   \n')).toBeUndefined()
  })
})

describe('what to tell someone to run', () => {
  it('turns a status probe into its login', () => {
    expect(signInCommand(CLI)).toBe('glab auth login')
  })

  it('names the tool itself when the probe is not a status check', () => {
    const aws: SdkConnectorAuth = {
      rung: 'cli',
      probe: { command: 'aws', args: ['sts', 'get-caller-identity'] }
    }
    expect(signInCommand(aws)).toBe('aws')
  })
})

describe('where to get a tool', () => {
  it('says something specific for a tool it knows', () => {
    expect(installHintFor('gh')).toMatch(/gh/)
  })

  it('stays honest about a tool it does not know', () => {
    expect(installHintFor('acmectl')).toContain('acmectl')
    expect(installHintFor('acmectl')).toContain('PATH')
  })
})
