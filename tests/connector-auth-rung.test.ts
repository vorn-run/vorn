import { describe, it, expect, vi, afterEach } from 'vitest'
import type { SdkConnectorAuth } from '../packages/shared/src/types'
import { setEnvPassthrough } from '../packages/server/src/process-utils'
import {
  borrowableNames,
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

/** What the connector declares reading, which is what it may borrow. */
const DECLARED = ['GITLAB_TOKEN', 'GITLAB_HOST']
const src = (auth: SdkConnectorAuth | undefined, declared: readonly string[] = DECLARED) => ({
  auth,
  declared
})

/** A tool that is on PATH and answers however the test says. */
const found = (stdout = '', stderr = '') => ({
  resolve: () => '/usr/bin/glab',
  run: vi.fn().mockResolvedValue({ stdout, stderr })
})

afterEach(() => {
  vi.unstubAllEnvs()
})

describe('what a connector is allowed to borrow', () => {
  it('passes a name it declares reading', () => {
    expect(borrowableNames(src(CLI, DECLARED))).toEqual(['GITLAB_TOKEN'])
  })

  it('hands over the name in the casing the connector declared it', () => {
    const loose: SdkConnectorAuth = { ...CLI, borrow: { env: ['gitlab_token'] } }
    expect(borrowableNames(src(loose))).toEqual(['GITLAB_TOKEN'])
  })

  it('refuses a name the connector never declared', () => {
    const greedy: SdkConnectorAuth = { ...CLI, borrow: { env: ['AWS_SECRET_ACCESS_KEY'] } }
    expect(borrowableNames(src(greedy, DECLARED))).toEqual([])
  })

  it('refuses a name that is stripped from every environment, declared or not', () => {
    const sneaky: SdkConnectorAuth = { ...CLI, borrow: { env: ['CLAUDE_CODE_SECRET'] } }
    expect(borrowableNames(src(sneaky, [...DECLARED, 'CLAUDE_CODE_SECRET']))).toEqual([])
  })

  it('refuses a credential name even when the manifest declares it, since both come from the pack', () => {
    const greedy: SdkConnectorAuth = {
      ...CLI,
      borrow: { env: ['ANTHROPIC_API_KEY', 'GITLAB_TOKEN'] }
    }
    expect(borrowableNames(src(greedy, ['ANTHROPIC_API_KEY', 'GITLAB_TOKEN']))).toEqual([
      'GITLAB_TOKEN'
    ])
  })

  it('ignores the agent passthrough list, which was never meant for a pack', () => {
    const greedy: SdkConnectorAuth = { ...CLI, borrow: { env: ['ANTHROPIC_API_KEY'] } }
    setEnvPassthrough(['ANTHROPIC_API_KEY'])
    try {
      expect(borrowableNames(src(greedy, ['ANTHROPIC_API_KEY']))).toEqual([])
    } finally {
      setEnvPassthrough([])
    }
  })

  it('lets an auth block the app itself wrote name a credential', () => {
    const gh: SdkConnectorAuth = {
      rung: 'cli',
      probe: { command: 'gh', args: ['auth', 'status'] },
      borrow: { env: ['GH_TOKEN'] }
    }
    expect(borrowableNames({ auth: gh, declared: ['GH_TOKEN'], trusted: true })).toEqual([
      'GH_TOKEN'
    ])
  })

  it('keeps a declared name out of the environment when nothing set it', () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    expect(borrowedEnv(src(CLI)).GITLAB_TOKEN).toBeUndefined()
  })

  it('hands over a declared name the shell already set', () => {
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    expect(borrowedEnv(src(CLI)).GITLAB_TOKEN).toBe('from-the-shell')
  })

  it('will not hand over an undeclared name even when the shell has one', () => {
    vi.stubEnv('AWS_SECRET_ACCESS_KEY', 'not-yours')
    const greedy: SdkConnectorAuth = { ...CLI, borrow: { env: ['AWS_SECRET_ACCESS_KEY'] } }
    expect(borrowedEnv(src(greedy)).AWS_SECRET_ACCESS_KEY).toBeUndefined()
  })
})

describe('asking a borrowed tool who you are', () => {
  it('reports the account the tool names', async () => {
    const deps = found('', 'Logged in to gitlab.com account javier (keyring)')
    expect(await probeAuth(src(CLI), deps)).toEqual({ ok: true, identity: 'javier' })
  })

  it('runs the probe the connector declared, with its own borrowed variables', async () => {
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const deps = found('Logged in as javier')
    await probeAuth(src(CLI), deps)
    const [file, args, options] = deps.run.mock.calls[0]
    expect(file).toBe('/usr/bin/glab')
    expect(args).toEqual(['auth', 'status'])
    expect(options.env.GITLAB_TOKEN).toBe('from-the-shell')
  })

  it('names the sign-in to run when the tool says no', async () => {
    const deps = { resolve: () => '/usr/bin/glab', run: vi.fn().mockRejectedValue(new Error('1')) }
    const report = await probeAuth(src(CLI), deps)
    expect(report.ok).toBe(false)
    expect(report.message).toContain('glab auth login')
  })

  it('offers a way to get the tool when it is the tool that is missing', async () => {
    const report = await probeAuth(src(CLI), { resolve: () => null, run: vi.fn() })
    expect(report.ok).toBe(false)
    expect(report.message).toContain('not installed')
    expect(report.installHint).toBeTruthy()
  })

  it('answers a rung this probe cannot speak to with nothing rather than a no', async () => {
    expect(await probeAuth(src({ rung: 'key', keys: ['token'] }))).toEqual({ ok: null })
    expect(await probeAuth(src(undefined))).toEqual({ ok: null })
  })

  it('calls a connector that needs no sign-in ready', async () => {
    const report = await probeAuth(src({ rung: 'none' }))
    expect(report.ok).toBe(true)
    expect(report.message).toContain('Nothing to sign in to')
  })
})

describe('what a borrowed tool hands over', () => {
  it('passes through a variable the shell already set, without running anything', async () => {
    vi.stubEnv('GITLAB_TOKEN', 'already-here')
    const deps = found('printed-token')
    expect(await borrowedSecrets(src(CLI), deps)).toEqual({ GITLAB_TOKEN: 'already-here' })
    expect(deps.run).not.toHaveBeenCalled()
  })

  it('asks the tool for a token when nothing set it', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    const deps = found('printed-token\n')
    expect(await borrowedSecrets(src(CLI), deps)).toEqual({ GITLAB_TOKEN: 'printed-token' })
    expect(deps.run.mock.calls[0][1]).toEqual(['auth', 'token'])
  })

  it('fills only the variable meant for the token, not every name borrowed', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    vi.stubEnv('GITLAB_HOST', '')
    const both: SdkConnectorAuth = {
      ...CLI,
      borrow: {
        env: ['GITLAB_HOST', 'GITLAB_TOKEN'],
        tokenArgs: ['auth', 'token'],
        tokenEnv: 'GITLAB_TOKEN'
      }
    }
    const borrowed = await borrowedSecrets(src(both), found('printed-token'))
    expect(borrowed).toEqual({ GITLAB_TOKEN: 'printed-token' })
    expect(borrowed.GITLAB_HOST).toBeUndefined()
  })

  it('gives the token to the first name asked for when none is singled out', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    const borrowed = await borrowedSecrets(src(CLI), found('printed-token'))
    expect(borrowed).toEqual({ GITLAB_TOKEN: 'printed-token' })
  })

  it('borrows nothing for a rung that does not borrow', async () => {
    const deps = found('printed-token')
    expect(await borrowedSecrets(src({ rung: 'key', keys: ['token'] }), deps)).toEqual({})
    expect(deps.run).not.toHaveBeenCalled()
  })

  it('borrows nothing the connector did not declare reading', async () => {
    vi.stubEnv('AWS_SECRET_ACCESS_KEY', 'not-yours')
    const greedy: SdkConnectorAuth = { ...CLI, borrow: { env: ['AWS_SECRET_ACCESS_KEY'] } }
    expect(await borrowedSecrets(src(greedy), found())).toEqual({})
  })

  it('leaves the child to fail on its own when the tool will not hand a token over', async () => {
    vi.stubEnv('GITLAB_TOKEN', '')
    const deps = { resolve: () => '/usr/bin/glab', run: vi.fn().mockRejectedValue(new Error('no')) }
    expect(await borrowedSecrets(src(CLI), deps)).toEqual({})
  })
})

describe('reading an identity out of what a tool printed', () => {
  it('prefers the account a tool names', () => {
    expect(identityFrom('github.com\n  Logged in to github.com account javier (keyring)')).toBe(
      'javier'
    )
  })

  it('reads the other phrasing tools use', () => {
    expect(identityFrom('Logged in to gitlab.com as javier.')).toBe('javier')
  })

  it('says nothing rather than printing whatever came first', () => {
    // `gh auth token` prints a credential; a first-line fallback would have rendered it as who you are.
    expect(identityFrom('ghp_0123456789abcdefghijklmnopqrstuvwxyz')).toBeUndefined()
    expect(identityFrom('{\n  "id": "abc",\n  "name": "Prod"\n}')).toBeUndefined()
  })

  it('does not read a refusal as a name', () => {
    expect(identityFrom('could not authenticate as anonymous')).toBeUndefined()
    expect(identityFrom('not signed in as anyone')).toBeUndefined()
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
