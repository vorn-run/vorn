import type { ShellSetup, ShellOptions } from './types'
import { writeShimDir } from './shim'

/**
 * zsh: ZDOTDIR is pointed at a shim directory whose files source the user's
 * own zsh files unchanged before installing precmd/preexec hooks.
 *
 * The redirect is how VS Code does it too — zsh offers no other way to inject
 * initialisation without editing the user's files, which we will not do.
 */

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

export function zshSetup(opts: ShellOptions): ShellSetup {
  const dir = writeShimDir('zsh', {
    '.zshenv': ZSHENV,
    '.zprofile': ZPROFILE,
    '.zshrc': ZSHRC
  })
  return {
    env: {
      ZDOTDIR: dir,
      VORN_USER_ZDOTDIR: opts.userZdotdir,
      VORN_MINIMAL_PROMPT: opts.minimalPrompt ? '1' : '0',
      // The gap is only worth it once commands are visually separated at all;
      // it rides along with the minimal prompt.
      VORN_BLOCK_GAP: opts.minimalPrompt ? '1' : ''
    },
    args: null
  }
}
