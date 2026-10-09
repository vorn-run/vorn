import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('electron', () => ({
  app: { getPath: () => '/Applications/Vorn.app/Contents/MacOS/Vorn', isPackaged: true }
}))

// Which directories exist and are writable is the machine's business, not the test's.
const writable = vi.hoisted(() => new Set<string>())
const files = vi.hoisted(() => new Map<string, string>())
vi.mock('node:fs', () => ({
  default: {
    accessSync: (dir: string) => {
      if (!writable.has(dir)) throw new Error('not writable')
    },
    constants: { W_OK: 2 },
    existsSync: () => false,
    mkdirSync: () => undefined,
    readFileSync: (file: string) => {
      const text = files.get(file)
      if (text === undefined) throw new Error('ENOENT')
      return text
    },
    writeFileSync: (file: string, text: string) => void files.set(file, text),
    chmodSync: () => undefined
  }
}))
vi.mock('../src/main/logger', () => ({ default: { warn: () => {}, error: () => {} } }))

import os from 'node:os'
import path from 'node:path'
import { onPath, refreshStaleShims, shimDirectory, shimScript } from '../src/main/cli-shim'

const MAC = {
  exe: '/Applications/Vorn.app/Contents/MacOS/Vorn',
  resources: '/Applications/Vorn.app/Contents/Resources'
}

describe('the vorn command the app writes', () => {
  it('runs the vorn binary inside the app on macOS, and opens the app when given nothing', () => {
    const script = shimScript(MAC, 'darwin')

    expect(script.startsWith('#!/bin/sh')).toBe(true)
    expect(script).toContain('exec open -a "/Applications/Vorn.app"')
    expect(script).toContain('exec "/Applications/Vorn.app/Contents/Resources/vornd/vorn" "$@"')
    expect(script).not.toContain('ELECTRON_RUN_AS_NODE')
  })

  it('names the AppImage rather than the mount it is unpacked into', () => {
    const script = shimScript(
      {
        exe: '/tmp/.mount_Vorn12/vorn',
        resources: '/tmp/.mount_Vorn12/resources',
        appImage: '/home/j/.local/lib/vorn/Vorn.AppImage'
      },
      'linux'
    )

    expect(script).toContain('/home/j/.local/lib/vorn/Vorn.AppImage')
    expect(script).not.toContain('.mount_Vorn12')
    // The command is only reachable from inside, so it is started from there.
    expect(script).toContain('process.env.APPDIR + "/resources/vornd/vorn"')
  })

  it('writes a batch file on Windows, starting the app when given nothing', () => {
    const script = shimScript(
      {
        exe: 'C:\\Users\\j\\AppData\\Local\\Programs\\Vorn\\Vorn.exe',
        resources: 'C:\\Users\\j\\AppData\\Local\\Programs\\Vorn\\resources'
      },
      'win32'
    )

    expect(script.startsWith('@echo off')).toBe(true)
    expect(script).toContain('if "%~1"=="" (')
    expect(script).not.toContain('ELECTRON_RUN_AS_NODE')
    // Written from a Mac in this test, and still a Windows path.
    expect(script).toContain('\\resources\\vornd\\vorn.exe" %*')
    expect(script).not.toContain('/resources/')
    expect(script.split('\n').every((line) => line === '' || line.endsWith('\r'))).toBe(true)
  })
})

describe('a command an older build installed', () => {
  const userBin = path.join(os.homedir(), '.local', 'bin', 'vorn')

  beforeEach(() => files.clear())

  it('is pointed at the native vorn when it ran the Node command line', () => {
    files.set(userBin, 'exec "$APP_EXE" "${RESOURCES}/server/cli.cjs" "$@"\n')
    files.set('/usr/local/bin/vorn', '#!/bin/sh\nexec something-else\n')

    expect(refreshStaleShims()).toEqual([userBin])
    expect(files.get(userBin)).toContain('/vornd/vorn" "$@"')
    expect(files.get('/usr/local/bin/vorn')).toBe('#!/bin/sh\nexec something-else\n')
  })
})

describe('where the command goes', () => {
  const userBin = path.join(os.homedir(), '.local', 'bin')
  const originalPath = process.env.PATH

  beforeEach(() => {
    writable.clear()
  })

  afterEach(() => {
    process.env.PATH = originalPath
  })

  it('prefers a writable directory the shell already searches', () => {
    writable.add('/usr/local/bin')
    writable.add(userBin)
    process.env.PATH = `${userBin}:/usr/bin`

    expect(shimDirectory()).toBe(userBin)
  })

  it('takes a writable directory over none when the shell searches neither', () => {
    writable.add('/usr/local/bin')
    process.env.PATH = '/usr/bin'

    expect(shimDirectory()).toBe('/usr/local/bin')
  })

  it('reads a Windows PATH entry whatever case it was written in', () => {
    process.env.PATH = 'C:\\Users\\J\\AppData\\Local\\Programs\\Vorn'

    expect(onPath('C:\\users\\j\\appdata\\local\\programs\\vorn', 'win32')).toBe(true)
    expect(onPath('C:\\users\\j\\appdata\\local\\programs\\vorn', 'linux')).toBe(false)
  })

  it('falls back to the one directory it may always create', () => {
    process.env.PATH = '/usr/bin'

    expect(shimDirectory()).toBe(userBin)
  })
})
