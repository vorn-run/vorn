/**
 * The wire format every shell integration emits.
 *
 * OSC 133 is the FinalTerm/FTCS protocol, the same one iTerm2, VS Code,
 * Windows Terminal, WezTerm, Kitty and Ghostty consume. A terminal cannot know
 * where one command ends and the next begins by looking at the byte stream —
 * only the shell knows, so it has to say so. That is why every terminal with
 * command blocks ships integration scripts rather than guessing.
 *
 *   A          prompt start
 *   B          prompt end / command line start
 *   C          command execution start (output follows)
 *   D;<code>   command finished, with exit code
 *
 * Not every shell can emit all four. A shell needs a pre-execution hook for C,
 * and a way to read the previous command's status for D's exit code; cmd.exe
 * has neither. Those shells emit what they can and the renderer fills the rest
 * in — see CommandBlockTracker.
 *
 * Two private sequences carry what OSC 133 has no field for. Both are namespaced
 * under 5522 so they cannot collide with a standard sequence:
 *
 *   5522;cwd;<path>     the directory the next command will run in
 *   5522;cmd;<base64>   the command text, encoded so newlines and control
 *                       characters survive the control-sequence framing
 *   5522;dur;<ms>       how long the command took, for shells that can only
 *                       report a command after the fact and would otherwise
 *                       show every block as instant
 */

export const OSC_PRIVATE = 5522

/** Shells that can report command boundaries, keyed by executable name. */
export type ShellFamily = 'zsh' | 'bash' | 'fish' | 'powershell' | 'cmd'

/**
 * What a shell is able to tell us. Recorded here rather than discovered at
 * runtime so the degraded cases are visible in one place instead of being
 * inferred from missing markers.
 */
export interface ShellCapabilities {
  /** Emits C, so the moment execution starts is known rather than assumed. */
  executionStart: boolean
  /** Emits D with a real exit code rather than a bare D. */
  exitCode: boolean
  /** Reports the command text, so a block can be titled. */
  commandText: boolean
}

export const CAPABILITIES: Record<ShellFamily, ShellCapabilities> = {
  zsh: { executionStart: true, exitCode: true, commandText: true },
  bash: { executionStart: true, exitCode: true, commandText: true },
  fish: { executionStart: true, exitCode: true, commandText: true },
  // PowerShell's prompt function runs between commands, so everything is
  // reported one prompt late — including the duration, which it can recover
  // from history rather than leaving at zero.
  powershell: { executionStart: false, exitCode: true, commandText: true },
  // cmd.exe can only decorate its PROMPT variable. There is no pre-execution
  // hook and no way to read the previous command's status from a prompt, so a
  // block is delimited but carries neither status nor title.
  cmd: { executionStart: false, exitCode: false, commandText: false }
}

/**
 * Which family a shell path belongs to, or null when we have no integration
 * for it and the session should run untouched.
 */
export function detectShellFamily(shellPath: string): ShellFamily | null {
  const name = shellPath
    .replace(/\\/g, '/')
    .split('/')
    .pop()!
    .toLowerCase()
    .replace(/\.exe$/, '')
  if (name === 'zsh') return 'zsh'
  if (name === 'bash') return 'bash'
  if (name === 'fish') return 'fish'
  if (name === 'pwsh' || name === 'powershell') return 'powershell'
  if (name === 'cmd') return 'cmd'
  return null
}
