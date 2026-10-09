import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import {
  findVornd,
  findWebClient,
  refusesDefault,
  runVornd,
  serveArgs
} from '../packages/server/src/vornd-binary'

/**
 * Where `vorn server serve` finds vornd and the web client, how it runs
 * vornd as the server, and the guard that keeps a run from source off this
 * machine's default data directory.
 */

describe('finding vornd', () => {
  const packaged = path.join('/app', 'Resources', 'server')
  const at = (dir: string, name: string): string => path.join(dir, name)

  it('looks beside the server in a packaged app, with the session holder next to it', () => {
    const seen: string[] = []
    const found = findVornd(packaged, {}, 'darwin', (f) => (seen.push(f), true))
    const dir = path.join('/app', 'Resources', 'vornd')
    expect(found).toEqual({ vornd: at(dir, 'vornd'), sessiond: at(dir, 'vorn-sessiond') })
    expect(seen[0]).toBe(at(dir, 'vornd'))
  })

  it('looks for the .exe names on Windows', () => {
    const dir = path.join('/app', 'Resources', 'vornd')
    expect(findVornd(packaged, {}, 'win32', () => true)).toEqual({
      vornd: at(dir, 'vornd.exe'),
      sessiond: at(dir, 'vorn-sessiond.exe')
    })
  })

  it('looks where build:core copies it in a checkout, then in cargo’s target directory', () => {
    const src = path.join('/repo', 'packages', 'server', 'src')
    const core = path.join('/repo', 'packages', 'core')
    const release = path.join(core, 'target', 'release', 'vornd')
    expect(findVornd(src, {}, 'linux', (f) => f === release)).toEqual({
      vornd: release,
      sessiond: null
    })
  })

  it('takes the binary VORN_VORND_PATH names first, and answers null without one', () => {
    const named = path.resolve('/opt/vornd/vornd')
    expect(findVornd(packaged, { VORN_VORND_PATH: named }, 'linux', () => true)?.vornd).toBe(named)
    expect(findVornd(packaged, {}, 'linux', () => false)).toBeNull()
  })

  it('finds the web client beside the server, packaged or in a checkout', () => {
    const packagedWeb = path.resolve('/app/Resources/web/dist')
    expect(findWebClient('/app/Resources/server', (d) => d === packagedWeb)).toBe(packagedWeb)
    expect(findWebClient('/app/Resources/server', () => false)).toBeNull()
  })
})

describe('running vornd as the server', () => {
  it('names the data directory, and what it has of the rest', () => {
    const binaries = { vornd: '/v/vornd', sessiond: '/v/vorn-sessiond' }
    expect(serveArgs(binaries, { dataDir: '/d', web: '/w', port: 4100, host: '0.0.0.0' })).toEqual([
      '--data-dir',
      '/d',
      '--sessiond',
      '/v/vorn-sessiond',
      '--web',
      '/w',
      '--port',
      '4100',
      '--host',
      '0.0.0.0'
    ])
    expect(serveArgs({ vornd: '/v/vornd', sessiond: null }, { dataDir: '/d' })).toEqual([
      '--data-dir',
      '/d'
    ])
  })

  it.runIf(process.platform !== 'win32')('passes vornd’s exit code back', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-fake-vornd-'))
    const fake = path.join(dir, 'vornd')
    fs.writeFileSync(fake, '#!/bin/sh\nexit 3\n', { mode: 0o755 })
    try {
      expect(await runVornd({ vornd: fake, sessiond: null }, [])).toBe(3)
      expect(await runVornd({ vornd: path.join(dir, 'missing'), sessiond: null }, [])).toBe(1)
    } finally {
      fs.rmSync(dir, { recursive: true, force: true })
    }
  })
})

describe('the default data directory', () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-home-'))

  it('is refused to a run from source unless allowed', () => {
    const vorn = path.join(home, '.vorn')
    expect(refusesDefault(vorn, { debug: true, home, env: {} })).toContain('default data directory')
    expect(
      refusesDefault(vorn, { debug: true, home, env: { VORN_ALLOW_DEFAULT_DATA_DIR: '1' } })
    ).toBeNull()
    expect(refusesDefault(vorn, { debug: false, home, env: {} })).toBeNull()
    expect(refusesDefault(path.join(home, 'other'), { debug: true, home, env: {} })).toBeNull()
  })

  it('counts as a run from source under the tests', () => {
    expect(refusesDefault(path.join(os.homedir(), '.vorn'))).not.toBeNull()
  })
})
