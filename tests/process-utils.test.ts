import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const mockExecFile = vi.fn()

vi.mock('node:child_process', async () => {
  const actual = await vi.importActual<typeof import('node:child_process')>('node:child_process')
  return {
    ...actual,
    execFileSync: vi.fn(() => {
      throw new Error('mock shell env')
    }),
    execFile: (...args: unknown[]) => mockExecFile(...args)
  }
})

describe('process-utils (server package)', () => {
  const originalEnv = process.env

  const TEST_ENV = {
    HOME: '/home/user',
    PATH: '/usr/bin',
    SHELL: '/bin/zsh',
    GITHUB_TOKEN: 'ghp_secret123',
    AWS_SECRET_ACCESS_KEY: 'aws-secret',
    OPENAI_API_KEY: 'sk-openai',
    CLAUDECODE: 'nested-session',
    EDITOR: 'vim',
    TERM: 'xterm-256color'
  }

  beforeEach(async () => {
    vi.resetModules()
    process.env = { ...TEST_ENV }
  })

  afterEach(() => {
    process.env = originalEnv
  })

  it('shellEscape skips quoting for simple safe strings', async () => {
    const { shellEscape } = await import('../packages/server/src/process-utils')
    // Simple flags — no quotes needed
    expect(shellEscape('--dangerously-skip-permissions')).toBe('--dangerously-skip-permissions')
    expect(shellEscape('--allow-all')).toBe('--allow-all')
    expect(shellEscape('-y')).toBe('-y')
    expect(shellEscape('-a')).toBe('-a')
    expect(shellEscape('never')).toBe('never')
    // Paths and values without spaces
    expect(shellEscape('/usr/bin/claude')).toBe('/usr/bin/claude')
    expect(shellEscape('--model=opus')).toBe('--model=opus')
    expect(shellEscape('file.txt')).toBe('file.txt')
  })

  it('shellEscape quotes strings with special characters', async () => {
    const { shellEscape } = await import('../packages/server/src/process-utils')
    // Strings with spaces
    expect(shellEscape('hello world')).toBe("'hello world'")
    expect(shellEscape('fix the bug')).toBe("'fix the bug'")
    // Strings with single quotes
    expect(shellEscape("it's")).toBe("'it'\\''s'")
    // Strings with shell metacharacters
    expect(shellEscape('echo $HOME')).toBe("'echo $HOME'")
    expect(shellEscape('a && b')).toBe("'a && b'")
    expect(shellEscape('test;rm -rf')).toBe("'test;rm -rf'")
    expect(shellEscape('$(whoami)')).toBe("'$(whoami)'")
    expect(shellEscape('`id`')).toBe("'`id`'")
    // Empty string
    expect(shellEscape('')).toBe("''")
  })

  it('shellEscape cmd flavor uses cmd.exe quoting on any platform', async () => {
    const { shellEscape } = await import('../packages/server/src/process-utils')
    // Multi-word → cmd.exe double quotes (not POSIX/PowerShell single quotes),
    // matching Node's shell:true which always runs cmd.exe on Windows.
    expect(shellEscape('# Workflow: Demo', 'cmd')).toBe('"# Workflow: Demo"')
    // Safe tokens still skip quoting.
    expect(shellEscape('-p', 'cmd')).toBe('-p')
    // cmd metacharacters are caret-escaped inside the quotes.
    expect(shellEscape('a "b" %PATH%', 'cmd')).toBe('"a ^"b^" ^%PATH^%"')
    // Empty → empty quoted arg.
    expect(shellEscape('', 'cmd')).toBe('""')
  })

  it('getSafeEnv filters sensitive vars', async () => {
    const { getSafeEnv } = await import('../packages/server/src/process-utils')
    const env = getSafeEnv()
    expect(env.HOME).toBe('/home/user')
    expect(env.PATH).toBe('/usr/bin')
    expect(env.EDITOR).toBe('vim')
    // Filtered
    expect(env.GITHUB_TOKEN).toBeUndefined()
    expect(env.AWS_SECRET_ACCESS_KEY).toBeUndefined()
    expect(env.OPENAI_API_KEY).toBeUndefined()
    expect(env.CLAUDECODE).toBeUndefined()
  })

  it('getDefaultShell returns SHELL or fallback', async () => {
    const { getDefaultShell } = await import('../packages/server/src/process-utils')
    const shell = getDefaultShell()
    expect(shell).toBe('/bin/zsh') // from TEST_ENV
  })

  it('getShellArgs returns platform-appropriate flags', async () => {
    const { getShellArgs } = await import('../packages/server/src/process-utils')
    if (process.platform === 'win32') {
      expect(getShellArgs()).toEqual([])
    } else {
      expect(getShellArgs()).toEqual(['-l'])
    }
  })

  describe('testSshConnection', () => {
    beforeEach(() => {
      mockExecFile.mockReset()
    })

    const host = {
      id: 'test-id',
      label: 'Test Host',
      hostname: 'example.com',
      user: 'ubuntu',
      port: 22
    }

    it('returns success when SSH echoes the marker', async () => {
      mockExecFile.mockImplementation(
        (
          _cmd: string,
          _args: string[],
          _opts: unknown,
          cb: (err: Error | null, stdout: string, stderr: string) => void
        ) => {
          cb(null, '__VORN_OK__\n', '')
          return { kill: vi.fn() }
        }
      )

      const { testSshConnection } = await import('../packages/server/src/process-utils')
      const result = await testSshConnection(host)
      expect(result.success).toBe(true)
      expect(result.message).toMatch(/Connected in \d+ms/)

      // Verify SSH args include BatchMode and StrictHostKeyChecking
      const args = mockExecFile.mock.calls[0][1] as string[]
      expect(args).toContain('BatchMode=yes')
      expect(args).toContain('StrictHostKeyChecking=accept-new')
      expect(args).toContain('ubuntu@example.com')
    })

    it('returns failure with stderr message on error', async () => {
      mockExecFile.mockImplementation(
        (
          _cmd: string,
          _args: string[],
          _opts: unknown,
          cb: (err: Error | null, stdout: string, stderr: string) => void
        ) => {
          cb(new Error('exit code 255'), '', 'Permission denied (publickey)')
          return { kill: vi.fn() }
        }
      )

      const { testSshConnection } = await import('../packages/server/src/process-utils')
      const result = await testSshConnection(host)
      expect(result.success).toBe(false)
      expect(result.message).toBe('Permission denied — check username and authentication method')
    })

    it('returns helpful message for host key verification failure', async () => {
      mockExecFile.mockImplementation(
        (
          _cmd: string,
          _args: string[],
          _opts: unknown,
          cb: (err: Error | null, stdout: string, stderr: string) => void
        ) => {
          cb(new Error('exit code 255'), '', 'Host key verification failed.')
          return { kill: vi.fn() }
        }
      )

      const { testSshConnection } = await import('../packages/server/src/process-utils')
      const result = await testSshConnection(host)
      expect(result.success).toBe(false)
      expect(result.message).toContain('known_hosts')
    })

    it('includes custom port and key path in args', async () => {
      mockExecFile.mockImplementation(
        (
          _cmd: string,
          _args: string[],
          _opts: unknown,
          cb: (err: Error | null, stdout: string, stderr: string) => void
        ) => {
          cb(null, '__VORN_OK__\n', '')
          return { kill: vi.fn() }
        }
      )

      const { testSshConnection } = await import('../packages/server/src/process-utils')
      await testSshConnection({ ...host, port: 2222, sshKeyPath: '/home/.ssh/id_ed25519' })

      const args = mockExecFile.mock.calls[0][1] as string[]
      expect(args).toContain('-p')
      expect(args).toContain('2222')
      expect(args).toContain('-i')
      expect(args).toContain('/home/.ssh/id_ed25519')
    })
  })
})

describe('normalizePath', () => {
  const mockRealpathSync = vi.fn()

  beforeEach(async () => {
    vi.resetModules()
    mockRealpathSync.mockReset()
    vi.doMock('node:fs', () => ({
      default: { realpathSync: mockRealpathSync }
    }))
  })

  it('strips trailing slashes', async () => {
    mockRealpathSync.mockImplementation((p: string) => p)
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('/app/')).toBe('/app')
  })

  it('strips multiple trailing slashes', async () => {
    mockRealpathSync.mockImplementation((p: string) => p)
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('/app///')).toBe('/app')
  })

  it('is a no-op for paths without trailing slashes', async () => {
    mockRealpathSync.mockImplementation((p: string) => p)
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('/app')).toBe('/app')
  })

  it('falls back to stripped path when realpathSync throws', async () => {
    mockRealpathSync.mockImplementation(() => {
      throw new Error('ENOENT')
    })
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('/nonexistent/path/')).toBe('/nonexistent/path')
  })

  it('resolves symlinks via realpathSync', async () => {
    mockRealpathSync.mockImplementation(() => '/private/var/data')
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('/var/data')).toBe('/private/var/data')
  })

  it('normalizes Windows-style paths case-insensitively', async () => {
    mockRealpathSync.mockImplementation((p: string) => p)
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('C:/Users/Javier/Repo/')).toBe('c:\\users\\javier\\repo')
  })

  it('preserves Windows drive roots', async () => {
    mockRealpathSync.mockImplementation((p: string) => p)
    const { normalizePath } = await import('../packages/server/src/process-utils')
    expect(normalizePath('C:\\')).toBe('c:\\')
  })
})
