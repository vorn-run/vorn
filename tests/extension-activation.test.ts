import { afterEach, describe, expect, it } from 'vitest'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { ExtensionActivation, InstalledConnectorPack } from '@vornrun/shared/types'
import {
  activationFor,
  matches,
  type ActivationSubject
} from '../packages/server/src/extensions/activation'

const temps: string[] = []

function worktree(files: string[] = []): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-activation-'))
  temps.push(dir)
  for (const file of files) {
    const path = join(dir, file)
    mkdirSync(join(path, '..'), { recursive: true })
    writeFileSync(path, '')
  }
  return dir
}

afterEach(() => {
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

function subject(over: Partial<ActivationSubject> = {}): ActivationSubject {
  return {
    worktreePath: worktree(),
    agent: 'claude',
    platform: 'darwin',
    remoteHost: () => 'github.com',
    ...over
  }
}

function pack(over: Partial<InstalledConnectorPack> = {}): InstalledConnectorPack {
  return {
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    path: '/tmp/review',
    installedAt: 0,
    bytes: 0,
    triggers: [],
    actions: [],
    env: [],
    contributes: {
      footers: [{ id: 'checks', title: 'Checks', every: 30 }],
      panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }]
    },
    permissions: ['terminal.read'],
    ...over
  }
}

describe('where a contribution shows', () => {
  it('shows where nothing was declared', () => {
    expect(matches(undefined, subject())).toBe(true)
    expect(matches({}, subject())).toBe(true)
  })

  it('reads workspaceContains as a path that is really there', () => {
    const withManifest = worktree(['package.json'])
    const bare = worktree()
    const predicate: ExtensionActivation = { workspaceContains: ['package.json', 'Cargo.toml'] }
    expect(matches(predicate, subject({ worktreePath: withManifest }))).toBe(true)
    expect(matches(predicate, subject({ worktreePath: bare }))).toBe(false)
  })

  // The manifest reader already refuses these; a pack whose file was edited by hand has not.
  it('refuses a path that leaves the worktree', () => {
    const dir = worktree(['package.json'])
    expect(
      matches({ workspaceContains: ['../package.json'] }, subject({ worktreePath: dir }))
    ).toBe(false)
    expect(matches({ workspaceContains: ['/etc/hosts'] }, subject({ worktreePath: dir }))).toBe(
      false
    )
    expect(
      matches({ workspaceContains: ['..\\package.json'] }, subject({ worktreePath: dir }))
    ).toBe(false)
  })

  it('matches the agent and the platform by name', () => {
    expect(matches({ agent: ['claude'] }, subject({ agent: 'claude' }))).toBe(true)
    expect(matches({ agent: ['codex'] }, subject({ agent: 'claude' }))).toBe(false)
    expect(matches({ platform: ['darwin'] }, subject({ platform: 'darwin' }))).toBe(true)
    expect(matches({ platform: ['win32'] }, subject({ platform: 'darwin' }))).toBe(false)
  })

  it('matches a remote host, whatever case it is written in', () => {
    expect(matches({ remoteHost: ['GitHub.com'] }, subject())).toBe(true)
    expect(matches({ remoteHost: ['gitlab.com'] }, subject())).toBe(false)
  })

  // Hiding here would cost an extension the repositories it was written for.
  it('widens when the remote cannot be read at all', () => {
    expect(matches({ remoteHost: ['github.com'] }, subject({ remoteHost: () => null }))).toBe(true)
  })

  it('asks for the remote only when a rule names one', () => {
    let asked = 0
    const counted = subject({
      remoteHost: () => {
        asked += 1
        return 'github.com'
      }
    })
    matches({ agent: ['claude'] }, counted)
    expect(asked).toBe(0)
    matches({ remoteHost: ['github.com'] }, counted)
    expect(asked).toBe(1)
  })
})

describe('what an extension shows on a session', () => {
  it('lists every contribution when nothing narrows it', () => {
    const activation = activationFor(pack(), subject())
    expect(activation).toEqual({
      active: true,
      panes: ['report'],
      footers: ['checks'],
      linkHandlers: []
    })
  })

  it('drops the contributions their own rule excludes', () => {
    const narrowed = pack({
      contributes: {
        footers: [{ id: 'checks', title: 'Checks', every: 30, when: { agent: ['codex'] } }],
        panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }]
      }
    })
    const activation = activationFor(narrowed, subject({ agent: 'claude' }))
    expect(activation.active).toBe(true)
    expect(activation.footers).toEqual([])
    expect(activation.panes).toEqual(['report'])
  })

  it('shows nothing when the extension itself does not activate', () => {
    const rusty = pack({ activates: { workspaceContains: ['Cargo.toml'] } })
    expect(activationFor(rusty, subject())).toEqual({
      active: false,
      panes: [],
      footers: [],
      linkHandlers: []
    })
  })

  it('contributes nothing from a connector', () => {
    expect(activationFor(pack({ kind: 'connector' }), subject()).active).toBe(false)
    expect(activationFor(pack({ kind: undefined }), subject()).active).toBe(false)
  })
})
