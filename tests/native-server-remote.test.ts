/**
 * A project on a remote host, reached by vornd itself over ssh through a real
 * server and the vornd it keeps: its files listed, read, written and stamped,
 * its git asked and changed, a worktree made, inventoried and removed. `ssh`
 * is a stub first on the login shell's PATH that runs each command on this
 * machine, so the "remote" project is a directory here. None of these calls
 * reaches the server.
 *
 * Runs where vornd and its session holder have been built, on a Unix.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { AppConfig } from '../packages/shared/src/types'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer
} from './helpers/real-server'

vi.setConfig({ testTimeout: 60_000, hookTimeout: 120_000 })

/** ssh as vornd runs it here: records its arguments, then runs the command it was given. */
const FAKE_SSH = `#!/bin/sh
us=$(printf '\\037')
line=
for a in "$@"; do line="$line$a$us"; done
printf '%s\\n' "$line" >> "LOG"
for last in "$@"; do :; done
exec /bin/sh -c "$last"
`

const made = (name: string): string =>
  fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-parity-${name}-`)))

function git(cwd: string, ...args: string[]): string {
  return execFileSync(
    'git',
    ['-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', ...args],
    { cwd, encoding: 'utf-8' }
  ).trim()
}

describe.runIf(runnable)('a project on a remote host, from vornd', () => {
  let server: RealServer
  let client: Watcher
  let repo = ''
  let log = ''
  let worktree = ''

  const call = <T = unknown>(method: string, params?: unknown): Promise<T> =>
    client.result<T>(method, params)
  const sshRuns = (): string[] => {
    try {
      return fs.readFileSync(log, 'utf-8').split('\n').filter(Boolean)
    } catch {
      return []
    }
  }

  beforeAll(async () => {
    const dirs = { home: made('home'), data: made('data'), work: made('work') }
    const bin = path.join(dirs.work, 'bin')
    fs.mkdirSync(bin)
    log = path.join(dirs.work, 'ssh-log')
    fs.writeFileSync(path.join(bin, 'ssh'), FAKE_SSH.replace('LOG', log), { mode: 0o755 })
    // Read by the login shell vornd asks for its environment, whichever shell it is.
    const prepend = `PATH="${bin}:$PATH"; export PATH\n`
    for (const file of ['.profile', '.bash_profile', '.bashrc', '.zprofile', '.zshrc']) {
      fs.writeFileSync(path.join(dirs.home, file), prepend)
    }
    repo = path.join(dirs.work, 'far')
    fs.mkdirSync(repo)
    git(repo, 'init', '-q', '-b', 'main')
    // The commit made over ssh runs where no identity may be set, as on CI.
    git(repo, 'config', 'user.name', 't')
    git(repo, 'config', 'user.email', 't@t')
    git(repo, 'config', 'commit.gpgsign', 'false')
    fs.writeFileSync(path.join(repo, 'README.md'), '# far\n')
    git(repo, 'add', '.')
    git(repo, 'commit', '-q', '-m', 'one')

    server = await startRealServer(dirs)
    client = await Watcher.open(server.vornd)
    const config = await call<AppConfig>('config:load')
    await call('config:save', {
      ...config,
      remoteHosts: [
        {
          id: 'box',
          label: 'Box',
          hostname: 'box.example',
          user: 'me',
          port: 2222,
          authMethod: 'agent'
        }
      ],
      projects: [{ name: 'far', path: repo, hostIds: ['box'], preferredAgents: ['claude'] }]
    })
  })

  afterAll(async () => {
    client?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('lists, reads, writes and stamps its files over ssh', async () => {
    const at = { remoteHostId: 'box' }
    const entries = await call<Array<{ name: string; isDirectory: boolean }>>('file:listDir', {
      dirPath: repo,
      ...at
    })
    expect(entries.map((e) => e.name)).toContain('README.md')
    const run = sshRuns().at(-1)!.split('\x1f')
    expect(run).toEqual(expect.arrayContaining(['BatchMode=yes', '-p', '2222', 'me@box.example']))

    const file = `${repo}/notes.txt`
    expect(await call('file:writeContent', { filePath: file, content: 'over ssh', ...at })).toEqual(
      {
        success: true
      }
    )
    expect(fs.readFileSync(file, 'utf-8')).toBe('over ssh')
    expect(await call('file:readContent', { filePath: file, ...at })).toBe('over ssh')
    expect(await call('file:readContent', { filePath: file, maxBytes: 4, ...at })).toBe(
      'over\n\n--- truncated ---'
    )
    expect(await call('file:stamp', { filePath: file, ...at })).toMatchObject({ size: 8 })
    expect(await call('file:stamp', { filePath: `${repo}/nothing`, ...at })).toBeNull()
    fs.rmSync(file)
  })

  it('asks and changes its git over ssh', async () => {
    const before = sshRuns().length
    const branches = await call<{ local: string[]; current: string; isGitRepo: boolean }>(
      'git:listBranches',
      repo
    )
    expect(branches).toEqual({ local: ['main'], current: 'main', isGitRepo: true })
    expect(sshRuns().length).toBeGreaterThan(before)

    const created = await call<{ worktreePath: string; branch: string }>('git:createWorktree', {
      projectPath: repo,
      branch: 'feat'
    })
    worktree = created.worktreePath
    expect(worktree.startsWith(`${path.dirname(repo)}/.vorn-worktrees/far/`)).toBe(true)
    expect(fs.existsSync(worktree)).toBe(true)
    expect(await call('git:getWorktreeBranch', worktree)).toBe('feat')

    fs.writeFileSync(path.join(worktree, 'change.txt'), 'x\n')
    expect(await call('git:worktreeDirty', worktree)).toBe(true)
    expect(
      await call('git:commit', { cwd: worktree, message: 'over ssh', includeUnstaged: true })
    ).toEqual({ success: true })
    expect(git(worktree, 'log', '-1', '--format=%s')).toBe('over ssh')
  })

  it('inventories and removes its worktrees over ssh', async () => {
    const inventory = await call<{
      projects: Array<{ remoteHostId: string | null; entries: Array<{ path: string }> }>
    }>('worktree:inventory', { projectPaths: [repo] })
    expect(inventory.projects[0].remoteHostId).toBe('box')
    expect(inventory.projects[0].entries.map((e) => e.path)).toContain(worktree)

    expect(
      await call('git:removeWorktree', { projectPath: repo, worktreePath: worktree, force: true })
    ).toBe(true)
    expect(fs.existsSync(worktree)).toBe(false)

    const health = (await (
      await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    ).json()) as {
      unexpectedForwards: Record<string, number>
      groups: Record<string, { forwarded?: number }>
    }
    expect(health.unexpectedForwards).toEqual({})
    for (const group of ['git', 'file', 'worktree']) {
      expect(health.groups[group]?.forwarded ?? 0).toBe(0)
    }
  })
})
