import { describe, it, expect, beforeEach } from 'vitest'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { detectShellFamily, CAPABILITIES } from '../packages/server/src/shell-integration/protocol'
import { bashSetup } from '../packages/server/src/shell-integration/bash'
import { fishSetup } from '../packages/server/src/shell-integration/fish'
import { powershellSetup } from '../packages/server/src/shell-integration/powershell'
import { cmdSetup } from '../packages/server/src/shell-integration/cmd'
import { resetShimCache } from '../packages/server/src/shell-integration/shim'

/**
 * Command boundaries for the shells beyond zsh. The markers are the same
 * FinalTerm/OSC 133 protocol every terminal with command blocks consumes; only
 * the way into each shell differs, and that is what these pin down.
 */

const OPTS = { minimalPrompt: true, userZdotdir: '/tmp/home' }

beforeEach(() => {
  resetShimCache()
})

describe('detectShellFamily', () => {
  it.each([
    ['/bin/zsh', 'zsh'],
    ['/usr/local/bin/bash', 'bash'],
    ['/opt/homebrew/bin/fish', 'fish'],
    ['C:\\Program Files\\PowerShell\\7\\pwsh.exe', 'powershell'],
    ['C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe', 'powershell'],
    ['C:\\Windows\\system32\\cmd.exe', 'cmd']
  ])('%s is %s', (shellPath, family) => {
    expect(detectShellFamily(shellPath)).toBe(family)
  })

  it('has no integration for an unknown shell, rather than guessing', () => {
    expect(detectShellFamily('/usr/bin/nu')).toBeNull()
    // /bin/sh is commonly dash, which has neither PROMPT_COMMAND nor a DEBUG
    // trap — treating it as bash would emit nothing and break the prompt.
    expect(detectShellFamily('/bin/sh')).toBeNull()
  })
})

describe('bash', () => {
  it('injects through --rcfile, with long options first', () => {
    const { args } = bashSetup(OPTS)
    // bash 3.2, which macOS still ships, rejects `-i --rcfile` outright.
    expect(args?.[0]).toBe('--rcfile')
    expect(args?.[2]).toBe('-i')
  })

  it('replays what a login shell reads, since --rcfile does not reach one', () => {
    const { args } = bashSetup(OPTS)
    const rc = fs.readFileSync(args![1], 'utf-8')
    expect(rc).toContain('/etc/profile')
    expect(rc).toContain('$HOME/.bash_profile')
    // bash reads .bashrc only when there is no profile at all. Anchored on the
    // $HOME/ prefix so this matches the statements, not the comments above them.
    expect(rc.indexOf('$HOME/.bash_profile')).toBeLessThan(rc.indexOf('$HOME/.bashrc'))
  })

  it('emits the full marker set', () => {
    const rc = fs.readFileSync(bashSetup(OPTS).args![1], 'utf-8')
    expect(rc).toContain(']133;A')
    expect(rc).toContain(']133;C')
    expect(rc).toContain(']133;D;%s')
    expect(rc).toContain(']5522;cmd;')
  })

  it('installs the DEBUG trap last, so its own setup is not reported', () => {
    const rc = fs.readFileSync(bashSetup(OPTS).args![1], 'utf-8')
    // The trap fires on every simple command. Installed early, the first thing
    // reported as the user's command is the shim's own PS1 assignment.
    expect(rc.indexOf("PS1=''")).toBeLessThan(rc.indexOf('trap '))
  })

  it('keeps the user PROMPT_COMMAND, and runs ours first for the exit status', () => {
    const rc = fs.readFileSync(bashSetup(OPTS).args![1], 'utf-8')
    expect(rc).toContain('PROMPT_COMMAND="__vorn_precmd;$PROMPT_COMMAND"')
  })
})

describe('fish', () => {
  it('installs as a vendor conf.d snippet found through XDG_DATA_DIRS', () => {
    const { env } = fishSetup(OPTS, '')
    const dir = env.XDG_DATA_DIRS.split(':')[0]
    expect(fs.existsSync(path.join(dir, 'fish/vendor_conf.d/vorn.fish'))).toBe(true)
  })

  it('prepends rather than replaces, keeping other vendors reachable', () => {
    const { env } = fishSetup(OPTS, '/usr/share:/opt/share')
    expect(env.XDG_DATA_DIRS.endsWith(':/usr/share:/opt/share')).toBe(true)
  })

  it('supplies the platform default when the user has none', () => {
    // Setting XDG_DATA_DIRS to only our directory would hide every system
    // completion fish would otherwise find.
    const { env } = fishSetup(OPTS, '')
    expect(env.XDG_DATA_DIRS).toContain('/usr/share')
  })

  it('captures the exit status before anything else can overwrite it', () => {
    const { env } = fishSetup(OPTS, '')
    const dir = env.XDG_DATA_DIRS.split(':')[0]
    const conf = fs.readFileSync(path.join(dir, 'fish/vendor_conf.d/vorn.fish'), 'utf-8')
    const handler = conf.slice(conf.indexOf('fish_postexec'))
    expect(handler.indexOf('set -l __vorn_status $status')).toBeLessThan(handler.indexOf('printf'))
  })
})

describe('powershell', () => {
  function decode(): string {
    const { args } = powershellSetup(OPTS)
    return Buffer.from(args![2], 'base64').toString('utf16le')
  }

  it('passes the setup encoded, so the execution policy cannot block it', () => {
    const { args } = powershellSetup(OPTS)
    // A .ps1 would be subject to the Restricted default on Windows clients.
    expect(args?.[0]).toBe('-NoExit')
    expect(args?.[1]).toBe('-EncodedCommand')
    expect(decode()).toContain('function Global:prompt')
  })

  it('reports the duration, which it can only know after the fact', () => {
    // Everything is reported one prompt late, so measuring in the renderer
    // would call every command instant.
    expect(decode()).toContain(']5522;dur;')
    expect(decode()).toContain('EndExecutionTime')
  })

  it('wraps the user prompt rather than discarding it when not minimal', () => {
    const wrapped = Buffer.from(
      powershellSetup({ ...OPTS, minimalPrompt: false }).args![2],
      'base64'
    ).toString('utf16le')
    expect(wrapped).toContain('$Global:__VornOriginalPrompt')
  })
})

describe('cmd', () => {
  it('brackets the prompt with markers via PROMPT', () => {
    const { env } = cmdSetup(OPTS, '')
    expect(env.PROMPT).toContain('$e]133;A$e\\')
    expect(env.PROMPT).toContain('$e]133;D$e\\')
    // No pre-execution hook exists, so there is no C and no exit code.
    expect(env.PROMPT).not.toContain('133;C')
  })

  it('wraps an existing prompt instead of replacing it', () => {
    const { env } = cmdSetup({ ...OPTS, minimalPrompt: false }, '$T$G')
    expect(env.PROMPT).toContain('$T$G')
  })

  it('needs no launch arguments', () => {
    expect(cmdSetup(OPTS, '').args).toBeNull()
  })
})

describe('declared capabilities', () => {
  it('records which shells cannot report execution start', () => {
    // The renderer relies on this being honest: it synthesises the missing
    // marker for these two and would produce no blocks at all otherwise.
    expect(CAPABILITIES.powershell.executionStart).toBe(false)
    expect(CAPABILITIES.cmd.executionStart).toBe(false)
    expect(CAPABILITIES.bash.executionStart).toBe(true)
    expect(CAPABILITIES.fish.executionStart).toBe(true)
    expect(CAPABILITIES.zsh.executionStart).toBe(true)
  })

  it('records that cmd can report neither status nor command text', () => {
    expect(CAPABILITIES.cmd.exitCode).toBe(false)
    expect(CAPABILITIES.cmd.commandText).toBe(false)
  })
})

// The scripts above are only claims until a real shell runs them.
const hasBash = process.platform !== 'win32' && spawnSync('bash', ['--version']).status === 0

describe.skipIf(!hasBash)('an interactive bash emits command boundaries', () => {
  it('reports the command, its output and a non-zero exit code', () => {
    const { args } = bashSetup(OPTS)
    const emptyHome = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bash-test-'))
    const result = spawnSync('bash', args!, {
      input: 'printf VORNPROBE\nfalse\nexit\n',
      encoding: 'utf-8',
      timeout: 10_000,
      env: { ...process.env, HOME: emptyHome, TERM: 'xterm-256color' }
    })
    const out = (result.stdout ?? '') + (result.stderr ?? '')
    expect(out).toContain(']133;A')
    expect(out).toContain(']133;C')
    expect(out).toContain(']133;D;0')
    // The exit code has to survive, or every block looks successful.
    expect(out).toContain(']133;D;1')
    expect(out).toContain(`]5522;cmd;${Buffer.from('printf VORNPROBE').toString('base64')}`)
    expect(out).toContain('VORNPROBE')
    fs.rmSync(emptyHome, { recursive: true, force: true })
  }, 15_000)
})
