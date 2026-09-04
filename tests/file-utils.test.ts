import { describe, it, expect, vi, beforeEach } from 'vitest'

const mockExecFile = vi.fn()
const mockExecFileSync = vi.fn()
vi.mock('node:child_process', () => ({
  execFile: (...args: unknown[]) => mockExecFile(...args),
  execFileSync: (...args: unknown[]) => mockExecFileSync(...args)
}))
vi.mock('node:util', () => ({
  promisify: (fn: unknown) => fn
}))

vi.mock('node:fs', () => ({
  default: {
    readdirSync: vi.fn(),
    statSync: vi.fn(),
    openSync: vi.fn(),
    readSync: vi.fn(),
    closeSync: vi.fn(),
    writeFileSync: vi.fn()
  }
}))

import fs from 'node:fs'
import {
  fileStamp,
  listDir,
  readFileContent,
  writeFileContent
} from '../packages/server/src/file-utils'
import type { RemoteHost } from '@vornrun/shared/types'

beforeEach(() => {
  vi.clearAllMocks()
})

describe('listDir', () => {
  it('returns sorted entries with directories first', async () => {
    mockExecFile.mockImplementation((_cmd: string, args: string[]) => {
      if (args[0] === 'rev-parse') return { stdout: '/repo-sort\n' }
      if (args[0] === 'ls-files') return { stdout: '' }
      return { stdout: '' }
    })

    vi.mocked(fs.readdirSync).mockReturnValue([
      { name: 'zebra.ts', isDirectory: () => false, isFile: () => true },
      { name: 'src', isDirectory: () => true, isFile: () => false },
      { name: 'alpha.ts', isDirectory: () => false, isFile: () => true },
      { name: 'lib', isDirectory: () => true, isFile: () => false }
    ] as unknown as ReturnType<typeof fs.readdirSync>)

    const result = await listDir('/repo-sort')
    expect(result.map((e) => e.name)).toEqual(['lib', 'src', 'alpha.ts', 'zebra.ts'])
    expect(result[0].isDirectory).toBe(true)
    expect(result[1].isDirectory).toBe(true)
    expect(result[2].isDirectory).toBe(false)
  })

  it('excludes .git, .DS_Store, and hidden files', async () => {
    mockExecFile.mockImplementation((_cmd: string, args: string[]) => {
      if (args[0] === 'rev-parse') return { stdout: '/repo-hidden\n' }
      return { stdout: '' }
    })

    vi.mocked(fs.readdirSync).mockReturnValue([
      { name: '.git', isDirectory: () => true, isFile: () => false },
      { name: '.DS_Store', isDirectory: () => false, isFile: () => true },
      { name: '.env', isDirectory: () => false, isFile: () => true },
      { name: '.github', isDirectory: () => true, isFile: () => false },
      { name: 'readme.md', isDirectory: () => false, isFile: () => true }
    ] as unknown as ReturnType<typeof fs.readdirSync>)

    const result = await listDir('/repo-hidden')
    expect(result.map((e) => e.name)).toEqual(['.github', 'readme.md'])
  })

  it('excludes git-ignored entries', async () => {
    mockExecFile.mockImplementation((_cmd: string, args: string[]) => {
      if (args[0] === 'rev-parse') return { stdout: '/repo-ignore\n' }
      if (args[0] === 'ls-files') return { stdout: 'node_modules\ndist\n' }
      return { stdout: '' }
    })

    vi.mocked(fs.readdirSync).mockReturnValue([
      { name: 'node_modules', isDirectory: () => true, isFile: () => false },
      { name: 'dist', isDirectory: () => true, isFile: () => false },
      { name: 'src', isDirectory: () => true, isFile: () => false }
    ] as unknown as ReturnType<typeof fs.readdirSync>)

    const result = await listDir('/repo-ignore')
    expect(result.map((e) => e.name)).toEqual(['src'])
  })

  it('returns empty array when readdirSync throws', async () => {
    mockExecFile.mockImplementation((_cmd: string, args: string[]) => {
      if (args[0] === 'rev-parse') return { stdout: '/repo-enoent\n' }
      return { stdout: '' }
    })

    vi.mocked(fs.readdirSync).mockImplementation(() => {
      throw new Error('ENOENT')
    })

    const result = await listDir('/repo-enoent')
    expect(result).toEqual([])
  })

  it('works when not in a git repo', async () => {
    mockExecFile.mockImplementation(() => {
      throw new Error('not a git repo')
    })

    vi.mocked(fs.readdirSync).mockReturnValue([
      { name: 'file.txt', isDirectory: () => false, isFile: () => true }
    ] as unknown as ReturnType<typeof fs.readdirSync>)

    const result = await listDir('/some/dir')
    expect(result).toEqual([{ name: 'file.txt', path: '/some/dir/file.txt', isDirectory: false }])
  })
})

describe('readFileContent', () => {
  it('returns file content as string', () => {
    const content = 'hello world'
    const buf = Buffer.from(content)

    vi.mocked(fs.statSync).mockReturnValue({ isFile: () => true, size: buf.length } as ReturnType<
      typeof fs.statSync
    >)
    vi.mocked(fs.openSync).mockReturnValue(42)
    vi.mocked(fs.readSync).mockImplementation(((_fd: number, buffer: NodeJS.ArrayBufferView) => {
      buf.copy(buffer as Buffer)
      return buf.length
    }) as typeof fs.readSync)

    const result = readFileContent('/test/file.txt')
    expect(result).toBe('hello world')
    expect(fs.closeSync).toHaveBeenCalledWith(42)
  })

  it('returns null for binary files (null bytes)', () => {
    const buf = Buffer.from([0x48, 0x65, 0x00, 0x6c, 0x6f]) // "He\0lo"

    vi.mocked(fs.statSync).mockReturnValue({ isFile: () => true, size: buf.length } as ReturnType<
      typeof fs.statSync
    >)
    vi.mocked(fs.openSync).mockReturnValue(42)
    vi.mocked(fs.readSync).mockImplementation(((_fd: number, buffer: NodeJS.ArrayBufferView) => {
      buf.copy(buffer as Buffer)
      return buf.length
    }) as typeof fs.readSync)

    const result = readFileContent('/test/binary.bin')
    expect(result).toBeNull()
  })

  it('returns null for directories', () => {
    vi.mocked(fs.statSync).mockReturnValue({
      isFile: () => false,
      size: 0
    } as ReturnType<typeof fs.statSync>)

    const result = readFileContent('/test/dir')
    expect(result).toBeNull()
  })

  it('adds truncation notice for large files', () => {
    const content = 'x'.repeat(100)
    const buf = Buffer.from(content)

    vi.mocked(fs.statSync).mockReturnValue({
      isFile: () => true,
      size: 1000
    } as ReturnType<typeof fs.statSync>)
    vi.mocked(fs.openSync).mockReturnValue(42)
    vi.mocked(fs.readSync).mockImplementation(((_fd: number, buffer: NodeJS.ArrayBufferView) => {
      const target = buffer as Buffer
      buf.copy(target, 0, 0, target.length)
      return target.length
    }) as typeof fs.readSync)

    const result = readFileContent('/test/big.txt', 50)
    expect(result).toContain('--- truncated (1000 bytes total) ---')
  })

  it('closes fd even when readSync throws', () => {
    vi.mocked(fs.statSync).mockReturnValue({ isFile: () => true, size: 100 } as ReturnType<
      typeof fs.statSync
    >)
    vi.mocked(fs.openSync).mockReturnValue(99)
    vi.mocked(fs.readSync).mockImplementation(() => {
      throw new Error('I/O error')
    })

    const result = readFileContent('/test/bad.txt')
    expect(result).toBeNull()
    expect(fs.closeSync).toHaveBeenCalledWith(99)
  })
})

describe('writeFileContent', () => {
  it('writes content locally and returns success', () => {
    vi.mocked(fs.writeFileSync).mockImplementation(() => undefined)

    const result = writeFileContent('/test/out.txt', 'hello\nworld')
    expect(result).toEqual({ success: true })
    expect(fs.writeFileSync).toHaveBeenCalledWith('/test/out.txt', 'hello\nworld', 'utf-8')
  })

  it('returns error on local write failure', () => {
    vi.mocked(fs.writeFileSync).mockImplementation(() => {
      throw new Error('EACCES: permission denied')
    })

    const result = writeFileContent('/locked/file.txt', 'data')
    expect(result.success).toBe(false)
    expect(result.error).toContain('EACCES')
  })

  it('writes via SSH stdin for a remote host (no argv-length pitfall)', () => {
    mockExecFileSync.mockReturnValue('')

    const remote: RemoteHost = {
      id: 'h1',
      label: 'example',
      hostname: 'example.com',
      user: 'alice',
      port: 22
    }
    const big = 'x'.repeat(300_000)
    const result = writeFileContent('/remote/file.txt', big, remote)

    expect(result).toEqual({ success: true })
    const [cmd, args, opts] = mockExecFileSync.mock.calls[0]
    expect(cmd).toBe('ssh')
    expect(args[args.length - 1]).toMatch(/^cat > .*\/remote\/file\.txt'?$/)
    expect((opts as { input: string }).input).toBe(big)
  })

  it('returns error when SSH exec throws', () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error('ssh: connect failed')
    })

    const remote: RemoteHost = {
      id: 'h1',
      label: 'example',
      hostname: 'example.com',
      user: 'alice',
      port: 22
    }
    const result = writeFileContent('/remote/file.txt', 'data', remote)

    expect(result.success).toBe(false)
    expect(result.error).toContain('ssh: connect failed')
  })
})

/**
 * The stamp a draft is compared against. Its only job is to be wrong rather than
 * absent when the file has moved, so every arm that could invent one is checked.
 */
describe('fileStamp', () => {
  const remote: RemoteHost = {
    id: 'h1',
    label: 'example',
    hostname: 'example.com',
    user: 'alice',
    port: 22
  }

  it('reports the size and mtime of a file', () => {
    vi.mocked(fs.statSync).mockReturnValue({
      isFile: () => true,
      size: 120,
      mtimeMs: 1_700_000_000_123
    } as never)

    expect(fileStamp('/repo/a.ts')).toEqual({ size: 120, mtimeMs: 1_700_000_000_123 })
  })

  it('floors the mtime, so a local stamp can be compared with a remote one', () => {
    // `stat` over ssh answers in whole seconds either side; keeping the
    // sub-second part here would read as changed against a stamp that never
    // had the precision to disagree.
    vi.mocked(fs.statSync).mockReturnValue({
      isFile: () => true,
      size: 1,
      mtimeMs: 1_700_000_000_999.7
    } as never)

    expect(fileStamp('/repo/a.ts')?.mtimeMs).toBe(1_700_000_000_999)
  })

  it('refuses to stamp a directory', () => {
    vi.mocked(fs.statSync).mockReturnValue({ isFile: () => false } as never)
    expect(fileStamp('/repo/src')).toBeNull()
  })

  it('answers null for a file that is gone rather than inventing one', () => {
    vi.mocked(fs.statSync).mockImplementation(() => {
      throw new Error('ENOENT')
    })
    // A stamp made up here would tell a draft the file is unchanged, and the
    // save would go through without asking.
    expect(fileStamp('/repo/gone.ts')).toBeNull()
  })

  it('asks a remote host with one call and reads seconds as milliseconds', () => {
    mockExecFileSync.mockReturnValue('120 1700000000\n')

    expect(fileStamp('/remote/a.ts', remote)).toEqual({
      size: 120,
      mtimeMs: 1_700_000_000_000
    })
    const [, args] = mockExecFileSync.mock.calls[0]
    // GNU first, BSD second: the flags are incompatible and there is no
    // portable spelling, so the shell tries both.
    expect(args[args.length - 1]).toContain("stat -c '%s %Y'")
    expect(args[args.length - 1]).toContain("stat -f '%z %m'")
  })

  it('answers null when the remote says something that is not a stamp', () => {
    mockExecFileSync.mockReturnValue('stat: illegal option\n')
    expect(fileStamp('/remote/a.ts', remote)).toBeNull()
  })

  it('answers null when the remote cannot be reached', () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error('ssh: connect failed')
    })
    expect(fileStamp('/remote/a.ts', remote)).toBeNull()
  })
})
