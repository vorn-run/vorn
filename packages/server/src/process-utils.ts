import { execFileSync, execFile, type ExecFileSyncOptions } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type { RemoteHost } from '@vornrun/shared/types'
// Constants only — this module is on the PTY spawn path, so it must not pull in
// anything that reaches the database.
import { BOOTSTRAP_ENV_VAR } from '@vornrun/shared/protocol'

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

/**
 * Resolved lazily and memoized: getUserShellEnv() spawns a login shell, and
 * doing that at import time made merely importing this module — as the pure
 * helpers' unit tests do — pay for a subprocess it never uses.
 */
let resolvedEnvCache: Record<string, string> | undefined

function resolvedEnv(): Record<string, string> {
  resolvedEnvCache ??= getUserShellEnv()
  return resolvedEnvCache
}

/**
 * The shell a terminal session should run.
 *
 * `configured` is the user's "Default Shell" setting and always wins when set.
 *
 * On Windows the fallback deliberately is not COMSPEC. That variable names the
 * interpreter Windows runs .bat files with — it is always cmd.exe and says
 * nothing about what a person wants to type into. cmd is also the weakest shell
 * we can integrate with: it has no pre-execution hook and no way to read the
 * previous command's status, so its blocks carry neither exit status nor
 * command text. PowerShell reports all of it, ships with every Windows 10 and
 * 11 install, and is what Windows Terminal itself opens by default.
 */
export function getDefaultShell(configured?: string): string {
  const chosen = configured?.trim()
  if (chosen) return chosen
  if (process.platform === 'win32') return findWindowsShell()
  return process.env.SHELL || '/bin/zsh'
}

function findWindowsShell(): string {
  // PowerShell 7 where it has been installed, then the Windows PowerShell that
  // is always present.
  const pathDirs = (process.env.PATH ?? '').split(path.delimiter).filter(Boolean)
  for (const dir of pathDirs) {
    const candidate = path.join(dir, 'pwsh.exe')
    if (fs.existsSync(candidate)) return candidate
  }
  const systemRoot = process.env.SystemRoot || process.env.windir || 'C:\\Windows'
  const windowsPowerShell = path.join(
    systemRoot,
    'System32',
    'WindowsPowerShell',
    'v1.0',
    'powershell.exe'
  )
  if (fs.existsSync(windowsPowerShell)) return windowsPowerShell
  // Only if neither exists, which should not happen on a supported Windows.
  return process.env.COMSPEC || 'cmd.exe'
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

/**
 * Markers an agent CLI leaves in the environment to describe the session it is
 * already inside.
 *
 * Vorn launches agents as subprocesses. When Vorn itself was started from an
 * agent session — `yarn dev` typed into one, or the app opened from it — these
 * propagate through the app into every agent it spawns, and each one then
 * believes it is a nested child of that original session. The visible symptom
 * is transcripts silently not being saved; the worse one is
 * CLAUDE_CODE_MESSAGING_SOCKET, which points a freshly launched agent at
 * another session's socket.
 *
 * The `CLAUDE_CODE_` prefix is stripped wholesale rather than by name because
 * this list has been wrong once already: CLAUDECODE alone was stripped while
 * five siblings went through untouched.
 */
export const STRIP_ENV_KEYS = ['CLAUDECODE']

/**
 * Stripped unconditionally, with no `envPassthrough` override.
 *
 * `BOOTSTRAP_ENV_VAR` is how the desktop hands its per-launch credential to the
 * server it spawns. The `SECRET_` prefix already puts it in
 * SENSITIVE_ENV_PREFIXES, but that list is overridable by name — and a
 * credential that authenticates as the owner has no business being forwardable
 * by configuration.
 *
 * Belt and braces rather than the control: the server deletes it from
 * `process.env` once read, so nothing can inherit it regardless of how it is
 * spawned. This list only matters for the window before that happens.
 */
const STRIP_ENV_PREFIXES = ['CLAUDE_CODE_', BOOTSTRAP_ENV_VAR]

// Compared uppercased. Windows environment variable names are case-insensitive
// and Node hands back whatever casing it enumerated, so a literal match would
// let `claudecode` through — and this list is meant to be absolute.
const STRIP_ENV_KEYS_UPPER = STRIP_ENV_KEYS.map((k) => k.toUpperCase())

/**
 * Whether a variable is stripped no matter who asks.
 *
 * `filterEnv` applies this while building an environment; a connector that
 * borrows a login names the variables it wants by hand, which walks around that
 * filter entirely. Exported so the borrow answers to the same list rather than
 * to a second copy of it that can drift.
 */
export function isAbsolutelyStrippedEnvName(name: string): boolean {
  const upper = name.toUpperCase()
  return (
    STRIP_ENV_KEYS_UPPER.includes(upper) || STRIP_ENV_PREFIXES.some((p) => upper.startsWith(p))
  )
}

/**
 * Exact variable names the user has opted into forwarding, uppercased.
 *
 * SENSITIVE_ENV_PREFIXES is the right default — a shell that exports a real
 * GITHUB_TOKEN should not hand it to every agent and script that happens to
 * run. But some setups need one of those names on purpose: pointing an agent
 * at a local Anthropic-compatible proxy needs ANTHROPIC_API_KEY, and the
 * prefix rule drops it while letting ANTHROPIC_BASE_URL through, leaving the
 * agent aimed at the proxy but authenticating as if it were not.
 *
 * Naming a variable here is a deliberate act, so it wins over the prefix rule.
 * STRIP_ENV_KEYS deliberately does not become overridable: those are stripped
 * to stop CLIs refusing to launch, and forwarding them breaks the session
 * rather than exposing anything.
 */
let envPassthrough: ReadonlySet<string> = new Set()

/**
 * Normalize whatever the config held into a comparable set.
 *
 * Typed as `unknown` on purpose: this value arrives from JSON.parse of a file a
 * user can hand-edit, so the declared `string[]` is a hope rather than a
 * guarantee. A bare string or a stray number in the list would otherwise throw
 * inside .map() — during server startup, where it takes the whole app down over
 * a typo in a settings file.
 */
export function normalizePassthrough(keys: unknown): ReadonlySet<string> {
  if (!Array.isArray(keys)) return new Set()
  return new Set(
    keys
      .filter((k): k is string => typeof k === 'string')
      .map((k) => k.trim().toUpperCase())
      .filter((k) => k.length > 0)
  )
}

export function setEnvPassthrough(keys: unknown): void {
  envPassthrough = normalizePassthrough(keys)
}

/**
 * The configured set, exposed so a test can assert what each env function
 * passes to filterEnv. getSafeEnv and getLaunchEnv themselves read the login
 * shell, which a unit test cannot stand up — the difference that matters is
 * which of these two sets each one hands over.
 */
export function getEnvPassthrough(): ReadonlySet<string> {
  return envPassthrough
}

/**
 * Pure form of the filter, so the rules can be tested without a login shell.
 */
export function filterEnv(
  source: Record<string, string | undefined>,
  passthrough: ReadonlySet<string>
): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [key, val] of Object.entries(source)) {
    if (val === undefined) continue
    const upper = key.toUpperCase()
    if (isAbsolutelyStrippedEnvName(key)) continue
    if (!passthrough.has(upper) && SENSITIVE_ENV_PREFIXES.some((p) => upper.startsWith(p))) continue
    env[key] = val
  }
  return env
}

/**
 * Environment for subprocesses the user never asked about — git, tailscale,
 * IDE and agent detection, file helpers. Always strict: a variable opted in so
 * an agent could authenticate has no business reaching `git ls-remote`.
 */
export function getSafeEnv(): Record<string, string> {
  return filterEnv(resolvedEnv(), new Set())
}

/**
 * Environment for an agent launch: the agent terminal, a headless agent, or a
 * workflow script node. Only these three forward `defaults.envPassthrough`.
 *
 * Not the plain shell session. A shell hands its environment to every command
 * typed into it for as long as it lives, which is far wider than "the agent I
 * configured a key for" — and wider than what the setting documents.
 *
 * Not the SSH session either: that connects to another machine, which is
 * precisely the widening this split exists to avoid.
 *
 * `VORN_DATA_DIR` is added rather than passed through, so anything launched from a
 * session can find the running server's port and credential files. They live in the
 * data directory, which `--data-dir` moves; without this, a tool started from a Vorn
 * terminal would look in `~/.vorn` and find a server that is not there.
 */
export function getLaunchEnv(): Record<string, string> {
  const env = filterEnv(resolvedEnv(), envPassthrough)
  if (launchDataDir) env.VORN_DATA_DIR = launchDataDir
  return env
}

/**
 * Told to us at startup rather than read from the database module.
 *
 * This module is on the PTY spawn path and must not import anything that reaches
 * the database — see the note at the top of the file — so the server pushes the
 * value in instead of us pulling it out.
 */
let launchDataDir: string | null = null

export function setLaunchDataDir(dir: string): void {
  launchDataDir = dir
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
