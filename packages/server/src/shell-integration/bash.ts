import type { ShellSetup, ShellOptions } from './types'
import { writeShimDir } from './shim'

/**
 * bash: launched with `--rcfile` pointed at a shim.
 *
 * There is no environment variable that injects initialisation into an
 * interactive bash — BASH_ENV applies only to non-interactive shells — so the
 * launch arguments are the only way in. `--rcfile` is ignored by a *login*
 * shell, which reads /etc/profile and ~/.bash_profile instead, so the `-l` we
 * normally pass is dropped and the shim replays those files itself in the same
 * order bash would have.
 *
 * The execution marker comes from a DEBUG trap rather than PS0, because PS0
 * arrived in bash 4.4 and macOS still ships 3.2. The trap has the further
 * advantage of exposing the command text in BASH_COMMAND, which PS0 cannot.
 */

function script(minimalPrompt: boolean): string {
  return `# Vorn shell integration for bash.

# Reproduce what a login shell reads, since --rcfile only reaches a non-login
# one: /etc/profile, then the first of the three personal profile files. bash
# does not read .bashrc for a login shell, so it is only a fallback for users
# who have no profile at all.
if [ -z "\${VORN_BASH_INIT-}" ]; then
  VORN_BASH_INIT=1
  [ -r /etc/profile ] && . /etc/profile
  __vorn_profile=""
  for __vorn_f in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [ -r "$__vorn_f" ]; then . "$__vorn_f"; __vorn_profile=1; break; fi
  done
  [ -z "$__vorn_profile" ] && [ -r "$HOME/.bashrc" ] && . "$HOME/.bashrc"
  unset __vorn_f __vorn_profile
fi

__vorn_executing=""
__vorn_first_prompt=1

__vorn_precmd() {
  # Must be the first statement: anything else overwrites the status we report.
  local __vorn_status=$?
  if [ -n "$__vorn_executing" ]; then
    __vorn_executing=""
    ${
      minimalPrompt
        ? `# A blank row under the output before the boundary, so the rule is not
    # pinned against the command's last line.
    printf '\\n'`
        : ':'
    }
    printf '\\033]133;D;%s\\007' "$__vorn_status"
  fi
${
  minimalPrompt
    ? `  # The row the boundary rule is drawn on. Skipped before the first prompt so
  # a session does not open on an empty line.
  [ -z "$__vorn_first_prompt" ] && printf '\\n'`
    : '  :'
}
  __vorn_first_prompt=""
  printf '\\033]5522;cwd;%s\\007' "$PWD"
  printf '\\033]133;A\\007'
}

__vorn_preexec() {
  # DEBUG fires before every simple command, including the ones PROMPT_COMMAND
  # runs and the ones completion runs. Only the first after a prompt is the
  # command the user actually typed.
  case "$BASH_COMMAND" in __vorn_precmd*) return ;; esac
  [ -n "\${COMP_LINE-}" ] && return
  [ -n "$__vorn_executing" ] && return
  __vorn_executing=1
  if command -v base64 >/dev/null 2>&1; then
    printf '\\033]5522;cmd;%s\\007' "$(printf '%s' "$BASH_COMMAND" | command base64 | command tr -d '\\n')"
  fi
  printf '\\033]133;C\\007'
}

# Prepended, not replaced: the user's own PROMPT_COMMAND keeps running, and
# ours has to go first to read the exit status before anything else clobbers it.
if [ -n "\${PROMPT_COMMAND-}" ]; then
  PROMPT_COMMAND="__vorn_precmd;$PROMPT_COMMAND"
else
  PROMPT_COMMAND="__vorn_precmd"
fi
${
  minimalPrompt
    ? `
# Nothing in place of the shell's prompt: the input bar already shows a caret
# and the command row is its own block heading, so a prompt would be a second
# caret for the same thing.
PS1=''
PS2='> '`
    : ''
}

# Installed last. The trap fires on every simple command, so anything this file
# still had to do would otherwise be reported as the user's first command.
trap '__vorn_preexec' DEBUG
`
}

export function bashSetup(opts: ShellOptions): ShellSetup {
  const dir = writeShimDir('bash', {
    'vorn-bashrc': script(opts.minimalPrompt)
  })
  return {
    env: {
      VORN_MINIMAL_PROMPT: opts.minimalPrompt ? '1' : '0'
    },
    // Long options first: bash 3.2, which macOS still ships, rejects
    // `-i --rcfile` outright. --rcfile is the injection point; -i restores the
    // interactivity that dropping -l would otherwise cost.
    args: ['--rcfile', `${dir}/vorn-bashrc`, '-i']
  }
}
