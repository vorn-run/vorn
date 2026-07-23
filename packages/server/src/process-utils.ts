import { execFileSync, execFile, type ExecFileSyncOptions } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type { RemoteHost } from '@vornrun/shared/types'

function getUserShellEnv(): Record<string, string> {
  if (process.platform === 'win32') return { ...process.env } as Record<string, string>
  try {
    const shell = process.env.SHELL || '/bin/zsh'
    const output = execFileSync(shell, ['-ilc', 'env'], {
      encoding: 'utf-8',
      timeout: 5000,
      stdio: ['pipe', 'pipe', 'pipe'] as ['pipe', 'pipe', 'pipe']
    })
    const env: Record<string, string> = {}
    for (const line of output.split('\n')) {
      const idx = line.indexOf('=')
      if (idx > 0) {
        env[line.substring(0, idx)] = line.substring(idx + 1)
      }
    }
    return env
  } catch {
    return { ...process.env } as Record<string, string>
  }
}

const resolvedEnv = getUserShellEnv()

export function getDefaultShell(): string {
  if (process.platform === 'win32') {
    return process.env.COMSPEC || 'powershell.exe'
  }
  return process.env.SHELL || '/bin/zsh'
}

/**
 * Quote an argument for a shell.
 *  - `'auto'`  — pick by platform, and on Windows by `getDefaultShell()`
 *    (PowerShell vs cmd.exe). Use for a PTY running the user's shell.
 *  - `'cmd'`   — force cmd.exe quoting. Use when the command runs through
 *    Node's `spawn(..., { shell: true })` on Windows, which always invokes
 *    `comspec || cmd.exe` regardless of the user's default shell — so quoting
 *    by `getDefaultShell()` (which falls back to PowerShell when COMSPEC is
 *    unset) would mismatch and leave args un-quoted / mangled.
 *  - `'posix'` — force POSIX single-quote quoting.
 */
export function shellEscape(s: string, flavor: 'auto' | 'posix' | 'cmd' = 'auto'): string {
  const isWin = flavor === 'cmd' || (flavor === 'auto' && process.platform === 'win32')
  // Skip quoting for simple safe strings (flags, paths without spaces, etc.)
  // On Windows, exclude % from safe chars to prevent env var expansion in cmd.exe.
  const safePattern = isWin ? /^[a-zA-Z0-9_./:=@+,-]+$/ : /^[a-zA-Z0-9_./:=@%+,-]+$/
  if (safePattern.test(s)) return s
  if (isWin) {
    // 'cmd' pins cmd.exe; 'auto' honors the user's PowerShell default.
    const shell = flavor === 'cmd' ? 'cmd.exe' : getDefaultShell().toLowerCase()
    if (shell.includes('powershell') || shell.includes('pwsh')) {
      return "'" + s.replace(/'/g, "''") + "'"
    }
    // cmd.exe: double quotes with caret escaping for ", %, and ^
    return '"' + s.replace(/["%^]/g, '^$&') + '"'
  }
  return "'" + s.replace(/'/g, "'\\''") + "'"
}

export function getShellArgs(): string[] {
  return process.platform === 'win32' ? [] : ['-l']
}

export const SENSITIVE_ENV_PREFIXES = [
  'AWS_SECRET',
  'AWS_SESSION',
  'GITHUB_TOKEN',
  'GH_TOKEN',
  'OPENAI_API',
  'ANTHROPIC_API',
  'GOOGLE_API',
  'STRIPE_',
  'DATABASE_URL',
  'DB_PASSWORD',
  'SECRET_',
  'PRIVATE_KEY',
  'NPM_TOKEN',
  'NODE_AUTH_TOKEN'
]

// Env vars to strip so agent CLIs don't refuse to launch (e.g. nested session detection)
export const STRIP_ENV_KEYS = ['CLAUDECODE']

export function getSafeEnv(): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [key, val] of Object.entries(resolvedEnv)) {
    if (val === undefined) continue
    if (SENSITIVE_ENV_PREFIXES.some((p) => key.toUpperCase().startsWith(p))) continue
    if (STRIP_ENV_KEYS.includes(key)) continue
    env[key] = val
  }
  return env
}

function isWindowsStylePath(p: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(p) || p.startsWith('\\\\')
}

/**
 * Normalize a filesystem path for reliable comparison.
 * Strips trailing slashes and resolves symlinks when the path exists.
 */
export function normalizePath(p: string): string {
  const pathImpl = isWindowsStylePath(p) ? path.win32 : path
  let result = pathImpl.normalize(p)
  const root = pathImpl.parse(result).root
  if (result !== root) {
    let end = result.length
    while (end > root.length && (result[end - 1] === '/' || result[end - 1] === '\\')) end--
    result = result.slice(0, end)
  }
  try {
    result = fs.realpathSync(result)
  } catch {
    // Path doesn't exist — use the normalized version
  }
  if (isWindowsStylePath(result) || isWindowsStylePath(p) || process.platform === 'win32') {
    result = result.toLowerCase()
  }
  return result
}

export interface SshTestResult {
  success: boolean
  message: string
  durationMs: number
}

export function testSshConnection(host: RemoteHost): Promise<SshTestResult> {
  return new Promise((resolve) => {
    const start = Date.now()
    const args = buildSshArgs(host, { connectTimeout: 5 })
    args.push('echo', '__VORN_OK__')

    const safetyTimer = setTimeout(() => {
      try {
        child.kill()
      } catch {
        /* already dead */
      }
    }, 12000)

    const child = execFile(
      'ssh',
      args,
      { timeout: 10000, env: getSafeEnv() },
      (err, stdout, stderr) => {
        clearTimeout(safetyTimer)
        const durationMs = Date.now() - start
        if (!err && stdout.includes('__VORN_OK__')) {
          resolve({ success: true, message: `Connected in ${durationMs}ms`, durationMs })
        } else {
          // Strip SSH warnings (e.g. "Warning: Permanently added ... to known hosts")
          const stderrClean = (stderr || '')
            .split('\n')
            .filter((line) => !line.startsWith('Warning:'))
            .join('\n')
            .trim()
          let msg = stderrClean || err?.message || 'Connection failed'
          if (msg.includes('Host key verification failed')) {
            msg = 'Host key changed — remove old entry from known_hosts or verify the server'
          } else if (msg.includes('Permission denied')) {
            msg = 'Permission denied — check username and authentication method'
          }
          resolve({ success: false, message: msg, durationMs })
        }
      }
    )
  })
}

/**
 * Build the SSH args prefix for a remote host (user@host, port, key, options).
 * Does NOT include the remote command — caller appends that.
 */
export function buildSshArgs(host: RemoteHost, opts?: { connectTimeout?: number }): string[] {
  const args: string[] = [
    '-o',
    `ConnectTimeout=${opts?.connectTimeout ?? 10}`,
    '-o',
    'BatchMode=yes',
    '-o',
    'StrictHostKeyChecking=accept-new'
  ]
  // SSH multiplexing (not available on Windows)
  if (process.platform !== 'win32') {
    const tmpDir = process.env.TMPDIR || '/tmp'
    args.push(
      '-o',
      'ControlMaster=auto',
      '-o',
      `ControlPath=${tmpDir}/vorn-ssh-%h-%p`,
      '-o',
      'ControlPersist=60'
    )
  }
  if (host.port !== 22) args.push('-p', String(host.port))
  if (host.sshKeyPath) args.push('-i', host.sshKeyPath)
  if (host.sshOptions) {
    args.push(...host.sshOptions.split(/\s+/).filter(Boolean))
  }
  args.push(`${host.user}@${host.hostname}`)
  return args
}

/**
 * Run a command synchronously on a remote host via SSH.
 * Equivalent to execFileSync('ssh', [...sshArgs, remoteCommand]).
 */
export function sshExecSync(
  host: RemoteHost,
  remoteCommand: string,
  opts?: { timeout?: number }
): string {
  const sshArgs = buildSshArgs(host)
  sshArgs.push(remoteCommand)
  const execOpts: ExecFileSyncOptions = {
    encoding: 'utf-8',
    stdio: ['pipe', 'pipe', 'pipe'],
    timeout: opts?.timeout ?? 15000,
    env: getSafeEnv()
  }
  return execFileSync('ssh', sshArgs, execOpts) as unknown as string
}
