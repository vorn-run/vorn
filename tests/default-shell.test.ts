import { describe, it, expect, vi, afterEach } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { getDefaultShell } from '../packages/server/src/process-utils'

/**
 * Which shell a terminal runs. The "Default Shell" setting was rendered in
 * Settings but read by nothing, so a user could not choose at all — and on
 * Windows the fallback was COMSPEC, which names the interpreter for .bat files
 * rather than anything a person wants to type into. It is always cmd.exe, the
 * one shell that can report neither exit status nor command text.
 */

const realPlatform = process.platform

function setPlatform(value: string): void {
  Object.defineProperty(process, 'platform', { value, configurable: true })
}

afterEach(() => {
  setPlatform(realPlatform)
  vi.restoreAllMocks()
})

describe('getDefaultShell', () => {
  it('uses the configured shell over anything else', () => {
    expect(getDefaultShell('/usr/local/bin/fish')).toBe('/usr/local/bin/fish')
  })

  it('ignores a blank setting rather than launching nothing', () => {
    expect(getDefaultShell('   ')).toBe(process.env.SHELL || '/bin/zsh')
  })

  it.skipIf(realPlatform === 'win32')('falls back to $SHELL elsewhere', () => {
    expect(getDefaultShell()).toBe(process.env.SHELL || '/bin/zsh')
  })
})

describe('getDefaultShell on Windows', () => {
  it('prefers PowerShell 7 when it is on PATH', () => {
    setPlatform('win32')
    vi.spyOn(fs, 'existsSync').mockImplementation((p) => String(p).endsWith('pwsh.exe'))
    vi.stubEnv('PATH', ['C:\\tools', 'C:\\ps'].join(path.delimiter))
    expect(getDefaultShell()).toContain('pwsh.exe')
  })

  it('falls back to the Windows PowerShell that always ships', () => {
    setPlatform('win32')
    vi.spyOn(fs, 'existsSync').mockImplementation((p) => String(p).includes('WindowsPowerShell'))
    vi.stubEnv('PATH', 'C:\\tools')
    vi.stubEnv('SystemRoot', 'C:\\Windows')
    expect(getDefaultShell()).toContain('powershell.exe')
  })

  it('does not pick cmd just because COMSPEC names it', () => {
    // COMSPEC is always cmd.exe on every Windows machine, so honouring it made
    // every Windows user get the weakest shell we support.
    setPlatform('win32')
    vi.spyOn(fs, 'existsSync').mockImplementation((p) => String(p).includes('WindowsPowerShell'))
    vi.stubEnv('COMSPEC', 'C:\\Windows\\system32\\cmd.exe')
    expect(getDefaultShell()).not.toContain('cmd.exe')
  })

  it('still honours cmd when the user asks for it', () => {
    setPlatform('win32')
    expect(getDefaultShell('C:\\Windows\\system32\\cmd.exe')).toContain('cmd.exe')
  })

  it('uses COMSPEC only when no PowerShell exists at all', () => {
    setPlatform('win32')
    vi.spyOn(fs, 'existsSync').mockReturnValue(false)
    vi.stubEnv('COMSPEC', 'C:\\Windows\\system32\\cmd.exe')
    expect(getDefaultShell()).toContain('cmd.exe')
  })
})

describe('the seeded setting', () => {
  it('matches the fallback rather than being COMSPEC in disguise', async () => {
    // The setting is seeded rather than left empty, so seeding it separately
    // pinned every user to whatever that seed said no matter what the fallback
    // preferred. Both now come from the same function.
    const { initTestDatabase, loadConfig } = await import('../packages/server/src/database')
    const close = initTestDatabase()
    try {
      expect(loadConfig().defaults.shell).toBe(getDefaultShell())
    } finally {
      close()
    }
  })
})
