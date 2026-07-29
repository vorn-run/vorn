import type { ShellSetup, ShellOptions } from './types'

/**
 * PowerShell: the integration is passed on the command line, base64-encoded.
 *
 * Dot-sourcing a .ps1 is subject to the execution policy, which is Restricted
 * by default on Windows clients and would make integration fail for exactly the
 * users least likely to diagnose it. `-EncodedCommand` is not a script file, so
 * the policy does not apply, and encoding sidesteps the quoting differences
 * between cmd.exe and the pty layer. Profiles are loaded before -Command runs,
 * so the user's own prompt is already in place for us to wrap.
 *
 * PowerShell has no pre-execution hook, so there is no C marker: everything is
 * reported by the `prompt` function, one prompt after the fact. The duration
 * would otherwise read as zero for every command, so it is recovered from the
 * history entry's own timestamps and sent alongside.
 */

function script(minimalPrompt: boolean): string {
  return `
$Global:__VornLastHistoryId = -1
$Global:__VornOriginalPrompt = $function:prompt

function Global:__VornExitCode {
  if ($? -eq $True) { return 0 }
  $h = Get-History -Count 1
  if ($Error[0].InvocationInfo.HistoryId -eq $h.Id) { return -1 }
  if ($null -eq $LastExitCode) { return 1 }
  return $LastExitCode
}

function Global:prompt {
  $code = __VornExitCode
  $h = Get-History -Count 1
  $out = ''
  if ($Global:__VornLastHistoryId -ne -1 -and $h -and $h.Id -ne $Global:__VornLastHistoryId) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($h.CommandLine)
    $out += "\`e]5522;cmd;$([Convert]::ToBase64String($bytes))\`a"
    $ms = [int]($h.EndExecutionTime - $h.StartExecutionTime).TotalMilliseconds
    $out += "\`e]5522;dur;$ms\`a"
    $out += "\`e]133;D;$code\`a"
${minimalPrompt ? '    $out += "\\n"\n' : ''}  }
  $out += "\`e]5522;cwd;$($executionContext.SessionState.Path.CurrentLocation)\`a"
  $out += "\`e]133;A\`a"
${
  minimalPrompt
    ? `  # Nothing in place of the prompt: the input bar already shows a caret and
  # each command is its own block heading. The string is still non-empty
  # because of the sequences, so PowerShell does not fall back to "PS>".`
    : '  $out += [string](& $Global:__VornOriginalPrompt)'
}
  $out += "\`e]133;B\`a"
  if ($h) { $Global:__VornLastHistoryId = $h.Id }
  return $out
}
`
}

export function powershellSetup(opts: ShellOptions): ShellSetup {
  // -EncodedCommand takes UTF-16LE, which is what PowerShell's own
  // documentation specifies and is not the default for Buffer.
  const encoded = Buffer.from(script(opts.minimalPrompt), 'utf16le').toString('base64')
  return {
    env: { VORN_MINIMAL_PROMPT: opts.minimalPrompt ? '1' : '0' },
    // -NoExit keeps the session interactive after the setup command runs.
    args: ['-NoExit', '-EncodedCommand', encoded]
  }
}
