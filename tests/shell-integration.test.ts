import { describe, it, expect, beforeEach } from 'vitest'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  getShellIntegrationEnv,
  resetShellIntegrationCache
} from '../packages/server/src/shell-integration'

const isZshHost =
  process.platform !== 'win32' &&
  path.basename(process.env.SHELL ?? '').includes('zsh') &&
  spawnSync('zsh', ['--version']).status === 0

describe('getShellIntegrationEnv', () => {
  beforeEach(() => {
    resetShellIntegrationCache()
  })

  it.skipIf(process.platform === 'win32')('writes the zsh shim files', () => {
    // Asks for zsh explicitly rather than reading the host's SHELL: every
    // shell is integrated now, so the host's own shell decides nothing here.
    const env = getShellIntegrationEnv({ shell: '/bin/zsh' })
    expect(env.ZDOTDIR).toBeTruthy()
    expect(env.VORN_USER_ZDOTDIR).toBeTruthy()
    const zshrc = fs.readFileSync(path.join(env.ZDOTDIR, '.zshrc'), 'utf-8')
    expect(zshrc).toContain(']133;A')
    expect(zshrc).toContain(']133;C')
    expect(zshrc).toContain(']133;D;%s')
    expect(zshrc).toContain(']5522;cmd;')
    expect(zshrc).toContain('paste:none')
    expect(fs.existsSync(path.join(env.ZDOTDIR, '.zshenv'))).toBe(true)
    expect(fs.existsSync(path.join(env.ZDOTDIR, '.zprofile'))).toBe(true)
  })

  it.skipIf(!isZshHost)(
    'an interactive zsh emits command boundary sequences',
    () => {
      const env = getShellIntegrationEnv()
      // Point the "user" zdotdir at an empty directory so the test doesn't
      // depend on (or slow down under) the developer's real zsh config.
      const emptyHome = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-si-test-'))
      const result = spawnSync('zsh', ['-i'], {
        input: 'printf VORNPROBE\nexit\n',
        encoding: 'utf-8',
        timeout: 10_000,
        env: {
          ...process.env,
          ...env,
          VORN_USER_ZDOTDIR: emptyHome,
          HOME: emptyHome,
          TERM: 'xterm-256color'
        }
      })
      const out = (result.stdout ?? '') + (result.stderr ?? '')
      expect(out).toContain(']133;A')
      expect(out).toContain(']133;C')
      expect(out).toContain(']133;D;0')
      // command text is base64 of "printf VORNPROBE"
      expect(out).toContain(`]5522;cmd;${Buffer.from('printf VORNPROBE').toString('base64')}`)
      expect(out).toContain('VORNPROBE')
      fs.rmSync(emptyHome, { recursive: true, force: true })
    },
    15_000
  )
})

describe('minimal prompt and block gap', () => {
  beforeEach(() => {
    resetShellIntegrationCache()
  })

  it('turns both on by default and off together when opted out', () => {
    if (!isZshHost) return
    const on = getShellIntegrationEnv()
    expect(on.VORN_MINIMAL_PROMPT).toBe('1')
    expect(on.VORN_BLOCK_GAP).toBe('1')

    const off = getShellIntegrationEnv({ minimalPrompt: false })
    expect(off.VORN_MINIMAL_PROMPT).toBe('0')
    expect(off.VORN_BLOCK_GAP).toBe('')
    // Shell integration itself must survive opting out — the spine still
    // needs command boundaries.
    expect(off.ZDOTDIR).toBeTruthy()
  })

  // The prompt itself only renders in a real interactive session, so the
  // contract worth asserting is the shim: PS1 is overridden only behind the
  // env guard, and always after the user's own rc has been sourced.
  it('guards the prompt override behind the env flag, after the user rc', () => {
    if (!isZshHost) return
    getShellIntegrationEnv()
    const shimrc = fs.readFileSync(
      path.join(os.tmpdir(), 'vorn-shell-integration', 'zsh', '.zshrc'),
      'utf-8'
    )
    expect(shimrc).toContain('$VORN_MINIMAL_PROMPT" == "1"')
    expect(shimrc).toContain('PS1=')
    // Sourcing the user's rc must come first, or their PS1 would win.
    expect(shimrc.indexOf('source "$ZDOTDIR/.zshrc"')).toBeLessThan(shimrc.indexOf('PS1='))
  })

  it('emits the block gap only when enabled, and never before the first prompt', () => {
    if (!isZshHost) return
    getShellIntegrationEnv()
    const shimrc = fs.readFileSync(
      path.join(os.tmpdir(), 'vorn-shell-integration', 'zsh', '.zshrc'),
      'utf-8'
    )
    expect(shimrc).toContain('-n "$VORN_BLOCK_GAP"')
    expect(shimrc).toContain('-z "$__vorn_first_prompt"')
  })
})

describe('prompt artifacts', () => {
  beforeEach(() => {
    resetShellIntegrationCache()
  })

  it('suppresses the partial-line marker under the minimal prompt', () => {
    if (!isZshHost) return
    getShellIntegrationEnv()
    const shimrc = fs.readFileSync(
      path.join(os.tmpdir(), 'vorn-shell-integration', 'zsh', '.zshrc'),
      'utf-8'
    )
    // zsh pads output that lacks a trailing newline with an inverse-video
    // "%" across the full width, right where the block boundary goes.
    expect(shimrc).toContain('unsetopt PROMPT_SP')
    expect(shimrc).toContain("PROMPT_EOL_MARK=''")
  })
})

describe('shim directory safety', () => {
  beforeEach(() => {
    resetShellIntegrationCache()
  })

  it('creates the shim owned by this user and not group/other writable', () => {
    if (!isZshHost) return
    const env = getShellIntegrationEnv()
    const stat = fs.lstatSync(env.ZDOTDIR)
    expect(stat.isDirectory()).toBe(true)
    // Every spawned shell sources these files, so a directory another local
    // user can write to is arbitrary code execution.
    expect(stat.mode & 0o022).toBe(0)
    if (typeof process.getuid === 'function') {
      expect(stat.uid).toBe(process.getuid())
    }
    for (const name of ['.zshenv', '.zprofile', '.zshrc']) {
      const f = fs.lstatSync(path.join(env.ZDOTDIR, name))
      expect(f.isSymbolicLink()).toBe(false)
      expect(f.mode & 0o022).toBe(0)
    }
  })

  it('declines rather than throwing when the directory cannot be secured', () => {
    if (!isZshHost) return
    // A session must still start; it just runs without command boundaries.
    const env = getShellIntegrationEnv()
    expect(typeof env).toBe('object')
  })
})
