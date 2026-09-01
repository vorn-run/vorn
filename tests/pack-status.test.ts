import { describe, expect, it } from 'vitest'
import type { InstalledConnectorPack } from '../packages/shared/src/types'
import {
  canAddConnection,
  describePackStatus,
  isNewerVersion,
  packStateFor
} from '../src/renderer/lib/pack-status'

const pack = (overrides: Partial<InstalledConnectorPack> = {}): InstalledConnectorPack => ({
  id: 'acme',
  name: 'Acme',
  version: '1.2.0',
  path: '/data/connectors/acme/1.2.0',
  installedAt: 0,
  bytes: 1024,
  triggers: [],
  actions: [],
  env: [],
  ...overrides
})

describe('isNewerVersion', () => {
  it('compares numeric segments left to right', () => {
    expect(isNewerVersion('1.3.0', '1.2.0')).toBe(true)
    expect(isNewerVersion('1.2.0', '1.3.0')).toBe(false)
    expect(isNewerVersion('2.0.0', '1.99.99')).toBe(true)
    expect(isNewerVersion('1.10.0', '1.9.0')).toBe(true)
  })

  it('is false for the same version', () => {
    expect(isNewerVersion('1.2.0', '1.2.0')).toBe(false)
  })

  it('ranks a release above the prereleases that led to it', () => {
    expect(isNewerVersion('1.3.0', '1.3.0-beta.1')).toBe(true)
    expect(isNewerVersion('1.3.0-beta.1', '1.3.0')).toBe(false)
    expect(isNewerVersion('1.3.0-beta.2', '1.3.0-beta.1')).toBe(true)
  })
})

describe('packStateFor', () => {
  it('is absent with nothing installed and nothing running', () => {
    expect(packStateFor({})).toEqual({ kind: 'absent' })
  })

  it('reports what is installed, with no update when the catalog matches', () => {
    expect(packStateFor({ installed: pack(), catalogVersion: '1.2.0' })).toEqual({
      kind: 'installed',
      version: '1.2.0'
    })
  })

  it('offers an update only when the catalog publishes something newer', () => {
    expect(packStateFor({ installed: pack(), catalogVersion: '1.3.0' })).toMatchObject({
      availableVersion: '1.3.0'
    })
    expect(packStateFor({ installed: pack(), catalogVersion: '1.1.0' })).not.toHaveProperty(
      'availableVersion'
    )
  })

  it('carries the rollback target when there is one', () => {
    expect(packStateFor({ installed: pack({ previousVersion: '1.1.0' }) })).toMatchObject({
      previousVersion: '1.1.0'
    })
  })

  it('lets a running install outrank what is on disk', () => {
    expect(
      packStateFor({
        installed: pack(),
        progress: { id: 'acme', phase: 'downloading', percent: 42 }
      })
    ).toEqual({ kind: 'installing', phase: 'downloading', percent: 42 })
  })

  it('lets a rejection outrank what is on disk, so a failed update says so', () => {
    expect(
      packStateFor({
        installed: pack(),
        progress: { id: 'acme', phase: 'failed', error: 'declares dependencies' }
      })
    ).toEqual({ kind: 'rejected', error: 'declares dependencies' })
  })

  it('names a rejection even when the failure carried no message', () => {
    expect(packStateFor({ progress: { id: 'acme', phase: 'failed' } })).toEqual({
      kind: 'rejected',
      error: 'The pack could not be installed'
    })
  })

  it('treats a finished install as installed rather than still running', () => {
    expect(
      packStateFor({ installed: pack(), progress: { id: 'acme', phase: 'installed' } })
    ).toMatchObject({ kind: 'installed' })
  })
})

describe('describePackStatus', () => {
  it('offers Install and spends no colour on a row with nothing on disk', () => {
    expect(describePackStatus({ kind: 'absent' })).toEqual({
      label: 'Install',
      detail: null,
      tone: 'idle',
      percent: null,
      action: 'install',
      busy: false
    })
  })

  it('names each phase and only carries a percent while downloading', () => {
    expect(
      describePackStatus({ kind: 'installing', phase: 'downloading', percent: 12 })
    ).toMatchObject({ label: 'Downloading', percent: 12, busy: true, action: null, tone: 'live' })
    expect(describePackStatus({ kind: 'installing', phase: 'verifying' })).toMatchObject({
      label: 'Verifying',
      percent: null,
      busy: true
    })
    expect(describePackStatus({ kind: 'installing', phase: 'installing' })).toMatchObject({
      label: 'Installing',
      percent: null
    })
  })

  it('has no percent for a download whose size was never advertised', () => {
    expect(describePackStatus({ kind: 'installing', phase: 'downloading' }).percent).toBeNull()
  })

  it('settles on the installed version, offering nothing further', () => {
    expect(describePackStatus({ kind: 'installed', version: '1.2.0' })).toEqual({
      label: 'Installed',
      detail: 'v1.2.0',
      tone: 'settled',
      percent: null,
      action: null,
      busy: false
    })
  })

  it('reads an available update as the one thing waiting on the person', () => {
    const view = describePackStatus({
      kind: 'installed',
      version: '1.2.0',
      availableVersion: '1.3.0'
    })
    expect(view.detail).toBe('v1.2.0 → 1.3.0 available')
    expect(view.action).toBe('update')
    expect(view.tone).toBe('blocked')
  })

  it('carries a rejection as danger with the reason kept intact', () => {
    const view = describePackStatus({ kind: 'rejected', error: 'declares scripts' })
    expect(view).toMatchObject({ label: "Couldn't install", tone: 'broken', action: 'retry' })
    expect(view.detail).toBe('declares scripts')
  })
})

describe('canAddConnection', () => {
  it('always arms a built-in, which is already in the process', () => {
    expect(canAddConnection({ kind: 'absent' }, { source: 'builtin' })).toBe(true)
  })

  it('arms a packaged connector once its files are on disk', () => {
    expect(canAddConnection({ kind: 'installed', version: '1.0.0' }, { source: 'catalog' })).toBe(
      true
    )
    expect(canAddConnection({ kind: 'installed', version: '1.0.0' }, { source: 'installed' })).toBe(
      true
    )
  })

  it('keeps a catalog entry with no pack reachable through its package name', () => {
    expect(canAddConnection({ kind: 'absent' }, { source: 'catalog', hasLegacyLaunch: true })).toBe(
      true
    )
    expect(
      canAddConnection({ kind: 'absent' }, { source: 'catalog', hasLegacyLaunch: false })
    ).toBe(false)
  })

  it('refuses a pack-only connector that is not installed or has been rejected', () => {
    expect(canAddConnection({ kind: 'absent' }, { source: 'installed' })).toBe(false)
    expect(canAddConnection({ kind: 'rejected', error: 'nope' }, { source: 'catalog' })).toBe(false)
  })
})
