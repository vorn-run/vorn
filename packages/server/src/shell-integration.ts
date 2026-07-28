import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { getDefaultShell, getSafeEnv } from './process-utils'
import log from './logger'

/**
 * Shell integration: command-boundary markers for the terminal.
 *
 * Local zsh sessions are launched with ZDOTDIR pointed at a shim directory.
 * The shim sources the user's own zsh files unchanged, then registers
 * precmd/preexec hooks that emit FinalTerm-style OSC 133 sequences:
 *
 *   A        prompt start
 *   C        command execution start (output follows)
 *   D;<code> command finished with exit code
 *
 * The command text itself travels in a private OSC 5522 sequence,
 * base64-encoded so multiline commands survive the control-sequence framing.
 * The renderer parses these to place block separators, exit-status
 * decorations, and jump targets — the pty byte stream is otherwise untouched.
 */

const SHIM_DIR = path.join(os.tmpdir(), 'vorn-shell-integration', 'zsh')

// Sourcing pattern: temporarily point ZDOTDIR at the user's directory while
// their file runs (so $ZDOTDIR-relative logic inside it behaves), capture any
// ZDOTDIR change it made, then restore the shim so the next stage still loads.
const ZSHENV = `# Vorn shell integration bootstrap — loads your own zsh files unchanged.
VORN_SHIM_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="\${VORN_USER_ZDOTDIR:-$HOME}"
[[ -f "$ZDOTDIR/.zshenv" ]] && builtin source "$ZDOTDIR/.zshenv"
VORN_USER_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$VORN_SHIM_ZDOTDIR"
`

const ZPROFILE = `if [[ -f "\${VORN_USER_ZDOTDIR:-$HOME}/.zprofile" ]]; then
  VORN_SHIM_ZDOTDIR="$ZDOTDIR"
  ZDOTDIR="\${VORN_USER_ZDOTDIR:-$HOME}"
  builtin source "$ZDOTDIR/.zprofile"
  VORN_USER_ZDOTDIR="$ZDOTDIR"
  ZDOTDIR="$VORN_SHIM_ZDOTDIR"
fi
`

const ZSHRC = `VORN_SHIM_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="\${VORN_USER_ZDOTDIR:-$HOME}"
# /etc/zshrc ran while ZDOTDIR pointed at the shim — repoint history at the
# user's file so their shell history is not diverted into the shim directory.
[[ "$HISTFILE" == "$VORN_SHIM_ZDOTDIR/.zsh_history" ]] && HISTFILE="$ZDOTDIR/.zsh_history"
[[ -f "$ZDOTDIR/.zshrc" ]] && builtin source "$ZDOTDIR/.zshrc"

# Restore the user's ZDOTDIR so .zlogin and nested shells resolve normally.
if [[ -n "$VORN_USER_ZDOTDIR" && "$VORN_USER_ZDOTDIR" != "$HOME" ]]; then
  ZDOTDIR="$VORN_USER_ZDOTDIR"
else
  unset ZDOTDIR
fi
unset VORN_SHIM_ZDOTDIR

autoload -Uz add-zsh-hook

# A single dim glyph in place of the shell's prompt. The command then reads
# as a heading above its own output rather than trailing a repeated
# user@host:path string, and the working directory is already shown in the
# session's own chrome. Opt out with the minimal-prompt setting, which
# leaves the user's prompt untouched.
if [[ "$VORN_MINIMAL_PROMPT" == "1" ]]; then
  # Empty, not a glyph. The input bar already shows a caret, and the command
  # row is marked by its own band, so a shell prompt would be a second caret
  # for the same thing — and it would sit in the buffer on every idle line.
  PS1=''
  PS2='%F{240}·%f '
  RPROMPT=''
  # zsh marks output that did not end in a newline with an inverse-video "%"
  # padded to the full width. That filler lands between the output and the
  # next command, which is exactly where the block boundary goes.
  unsetopt PROMPT_SP
  PROMPT_EOL_MARK=''
fi

__vorn_executing=""
__vorn_first_prompt=1
__vorn_precmd() {
  local __vorn_status=$?
  if [[ -n "$__vorn_executing" ]]; then
    __vorn_executing=""
    # A blank row under the output before the boundary is placed, so the rule
    # is not pinned against the command's last line.
    [[ -n "$VORN_BLOCK_GAP" ]] && print ""
    printf '\\033]133;D;%s\\007' "$__vorn_status"
  fi
  # The row the boundary rule is drawn on. xterm's grid has no inter-row
  # spacing, so the gap has to be a real line; printing it here keeps it
  # correct through scrollback, reflow and copy. Skipped before the first
  # prompt so a session does not open on an empty line.
  if [[ -n "$VORN_BLOCK_GAP" && -z "$__vorn_first_prompt" ]]; then
    print ""
  fi
  __vorn_first_prompt=""
  # Nothing is printed for the block's context here. Anything written at the
  # prompt appears before a command has been typed, so an idle session ends on
  # a stray line belonging to a block that may never run. Per-block context is
  # drawn by the renderer once the command finishes instead.
  printf '\\033]5522;cwd;%s\\007' "$PWD"
  printf '\\033]133;A\\007'
}
__vorn_preexec() {
  __vorn_executing=1
  if command -v base64 >/dev/null 2>&1; then
    printf '\\033]5522;cmd;%s\\007' "$(printf '%s' "$1" | command base64 | command tr -d '\\n')"
  fi
  printf '\\033]133;C\\007'
}
add-zsh-hook precmd __vorn_precmd
add-zsh-hook preexec __vorn_preexec

# Render pasted text like typed input instead of highlighted.
zle_highlight+=(paste:none)

# The command line in bold. This is what separates a block's heading from its
# output — weight, rather than a background band, which reads as a selection.
if [[ "$VORN_MINIMAL_PROMPT" == "1" ]]; then
  zle_highlight+=(default:bold)
fi
`

let shimReady = false

/**
 * Refuse a shim directory we do not exclusively own.
 *
 * On Linux os.tmpdir() is the shared /tmp, so another local user can
 * pre-create this path and own it — the sticky bit protects the entries in
 * /tmp, not the contents of a directory someone else created. Every shell we
 * spawn sources these files, so a planted .zshrc (or a symlink pointing at
 * one) would be arbitrary code execution as the user running Vorn.
 */
function shimDirIsSafe(dir: string): boolean {
  const stat = fs.lstatSync(dir)
  if (!stat.isDirectory()) return false
  // Not owned by us, or writable by group/other.
  if (typeof process.getuid === 'function' && stat.uid !== process.getuid()) return false
  return (stat.mode & 0o022) === 0
}

function ensureShimFiles(): void {
  if (shimReady) return
  fs.mkdirSync(SHIM_DIR, { recursive: true, mode: 0o700 })
  if (!shimDirIsSafe(SHIM_DIR)) {
    throw new Error(
      `refusing to use shim directory not exclusively owned by this user: ${SHIM_DIR}`
    )
  }
  // mkdir's mode is masked by umask and ignored when the directory already
  // exists, so set it explicitly.
  fs.chmodSync(SHIM_DIR, 0o700)
  const write = (name: string, contents: string): void => {
    const file = path.join(SHIM_DIR, name)
    // Truncate through any pre-existing symlink rather than following it.
    fs.rmSync(file, { force: true })
    fs.writeFileSync(file, contents, { mode: 0o600 })
  }
  write('.zshenv', ZSHENV)
  write('.zprofile', ZPROFILE)
  write('.zshrc', ZSHRC)
  shimReady = true
}

/**
 * Env additions that enable shell integration for a local pty.
 * Returns {} when the default shell isn't zsh (or the shim can't be written),
 * in which case the session runs exactly as before.
 */
export function getShellIntegrationEnv(
  opts: { minimalPrompt?: boolean } = {}
): Record<string, string> {
  if (process.platform === 'win32') return {}
  if (path.basename(getDefaultShell()) !== 'zsh') return {}
  try {
    ensureShimFiles()
  } catch (err) {
    log.warn(`[shell-integration] failed to write zsh shim: ${String(err)}`)
    return {}
  }
  // Default on: the terminal is drawn as command blocks, and the shell's own
  // prompt fights that. The setting exists for anyone who wants theirs back.
  const minimal = opts.minimalPrompt !== false
  return {
    ZDOTDIR: SHIM_DIR,
    VORN_USER_ZDOTDIR: getSafeEnv().ZDOTDIR ?? os.homedir(),
    VORN_MINIMAL_PROMPT: minimal ? '1' : '0',
    // The gap is only worth it once commands are visually separated at all;
    // it rides along with the minimal prompt.
    VORN_BLOCK_GAP: minimal ? '1' : ''
  }
}

/** Test hook: forget the cached shim state. */
export function resetShellIntegrationCache(): void {
  shimReady = false
  executableCache = null
}

// --- Executable listing (intent bar command completion) ---

let executableCache: { names: string[]; at: number } | null = null
const EXECUTABLE_CACHE_MS = 60_000

/**
 * Names of every file on PATH, deduplicated and sorted. Used by the intent
 * bar to complete the command token. The scan result is cached: PATH contents
 * change rarely, and readdir over a dozen bin dirs is not free.
 */
export function listShellExecutables(): string[] {
  if (executableCache && Date.now() - executableCache.at < EXECUTABLE_CACHE_MS) {
    return executableCache.names
  }
  const pathVar = getSafeEnv().PATH ?? process.env.PATH ?? ''
  const names = new Set<string>()
  for (const dir of pathVar.split(path.delimiter)) {
    if (!dir) continue
    try {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) continue
        names.add(entry.name)
      }
    } catch {
      // unreadable PATH entry — skip
    }
  }
  const sorted = [...names].sort()
  executableCache = { names: sorted, at: Date.now() }
  return sorted
}
