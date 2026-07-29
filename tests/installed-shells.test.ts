import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import path from 'node:path'

const statSync = vi.hoisted(() => vi.fn())
const execFileSync = vi.hoisted(() => vi.fn())

vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, default: { ...(actual.default as object), statSync }, statSync }
})
vi.mock('node:child_process', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, execFileSync }
})

import {
  listInstalledShells,
  resetInstalledShellCache
} from '../packages/server/src/shell-integration/installed'

/**
 * Paths are built the way the code builds them. Faking process.platform does
 * not switch Node's path module to Windows semantics, so hard-coded backslash
 * literals would never match on a POSIX test host.
 */
const WIN = {
  powershell: path.join('C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe'),
  cmd: path.join('C:\\Windows', 'System32', 'cmd.exe'),
  gitBash: 'C:\\Program Files\\Git\\bin\\bash.exe',
  // No drive letter: path.delimiter is still ':' on a POSIX test host, so
  // "C:\\bin" would split at the colon into two bogus directories.
  pwshOnPath: path.join('\\tools', 'pwsh.exe')
}

/**
 * Detecting what is on the machine, so choosing a shell does not mean knowing
 * its path by heart — and so the capability differences are visible at the
 * moment the choice is made.
 */

const realPlatform = process.platform

function setPlatform(value: string): void {
  Object.defineProperty(process, 'platform', { value, configurable: true })
}

/** Only the given paths exist. */
function existing(paths: string[]): void {
  const set = new Set(paths.map((p) => p.toLowerCase()))
  statSync.mockImplementation((p: string) => {
    if (!set.has(String(p).toLowerCase())) throw new Error('ENOENT')
    return { isFile: () => true }
  })
}

beforeEach(() => {
  resetInstalledShellCache()
  statSync.mockReset()
  execFileSync.mockReset()
  execFileSync.mockReturnValue('')
})

afterEach(() => {
  setPlatform(realPlatform)
  resetInstalledShellCache()
  vi.unstubAllEnvs()
})

describe('listInstalledShells on a POSIX host', () => {
  beforeEach(() => {
    setPlatform('darwin')
    vi.stubEnv('PATH', '/usr/bin:/opt/homebrew/bin')
  })

  it('finds shells on PATH and reads their versions', () => {
    existing(['/opt/homebrew/bin/fish'])
    execFileSync.mockReturnValue('fish, version 4.8.1\n')
    const found = listInstalledShells()
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({ family: 'fish', name: 'fish', version: '4.8.1' })
  })

  it('finds a shell that is not on PATH at all', () => {
    // /bin is deliberately absent from PATH above.
    existing(['/bin/zsh'])
    const found = listInstalledShells()
    expect(found.map((s) => s.path)).toContain('/bin/zsh')
  })

  it('lists the same shell once when PATH and a known location agree', () => {
    vi.stubEnv('PATH', '/bin')
    existing(['/bin/zsh'])
    expect(listInstalledShells().filter((s) => s.family === 'zsh')).toHaveLength(1)
  })

  it('orders the list best-first, so it reads as a recommendation', () => {
    vi.stubEnv('PATH', '/usr/bin')
    existing(['/usr/bin/zsh', '/usr/bin/bash', '/usr/bin/fish'])
    expect(listInstalledShells().map((s) => s.family)).toEqual(['zsh', 'fish', 'bash'])
  })

  it('still lists a shell whose version cannot be read', () => {
    existing(['/opt/homebrew/bin/fish'])
    execFileSync.mockImplementation(() => {
      throw new Error('timed out')
    })
    const found = listInstalledShells()
    expect(found[0].version).toBeNull()
    expect(found[0].path).toBe('/opt/homebrew/bin/fish')
  })

  it('caches, so opening settings does not re-spawn every shell', () => {
    existing(['/opt/homebrew/bin/fish'])
    listInstalledShells()
    const callsAfterFirst = execFileSync.mock.calls.length
    listInstalledShells()
    expect(execFileSync.mock.calls.length).toBe(callsAfterFirst)
  })
})

describe('listInstalledShells on Windows', () => {
  beforeEach(() => {
    setPlatform('win32')
    vi.stubEnv('PATH', 'C:\\bin')
    vi.stubEnv('SystemRoot', 'C:\\Windows')
  })

  it('finds the PowerShell and cmd that ship with Windows', () => {
    existing([WIN.powershell, WIN.cmd])
    expect(listInstalledShells().map((s) => s.family)).toEqual(['powershell', 'cmd'])
  })

  it('finds Git for Windows bash, which is usually off PATH', () => {
    existing([WIN.gitBash])
    expect(listInstalledShells().map((s) => s.family)).toEqual(['bash'])
  })

  it('puts PowerShell ahead of cmd', () => {
    // cmd is the weakest shell we integrate with, so it must not read as the
    // recommended option.
    existing([WIN.cmd, WIN.powershell])
    expect(listInstalledShells()[0].family).toBe('powershell')
  })

  it('appends .exe when scanning PATH', () => {
    vi.stubEnv('PATH', '\\tools')
    existing([WIN.pwshOnPath])
    expect(listInstalledShells().map((s) => s.path)).toContain(WIN.pwshOnPath)
  })
})

describe('what each shell can report', () => {
  beforeEach(() => {
    setPlatform('win32')
    vi.stubEnv('PATH', 'C:\\bin')
    vi.stubEnv('SystemRoot', 'C:\\Windows')
  })

  it('says cmd carries neither exit status nor command name', () => {
    existing([WIN.cmd])
    expect(listInstalledShells()[0].blocks).toEqual({
      level: 'limited',
      limitation: 'No exit status or command name on blocks'
    })
  })

  it('says PowerShell reports everything, but only once a command ends', () => {
    existing([WIN.powershell])
    expect(listInstalledShells()[0].blocks.level).toBe('partial')
  })

  it('claims nothing missing for a shell with a pre-execution hook', () => {
    existing([WIN.gitBash])
    expect(listInstalledShells()[0].blocks).toEqual({ level: 'full', limitation: null })
  })
})
