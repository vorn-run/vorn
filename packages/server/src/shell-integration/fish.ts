import type { ShellSetup, ShellOptions } from './types'
import { writeShimDir } from './shim'

/**
 * fish: a vendor conf.d snippet, found through XDG_DATA_DIRS.
 *
 * fish autoloads `<dir>/fish/vendor_conf.d/*.fish` for every entry in
 * XDG_DATA_DIRS. It is the mechanism fish provides for exactly this purpose, so
 * unlike bash and zsh nothing has to be redirected or replayed — the user's own
 * config loads normally, afterwards, and is never read by us.
 *
 * fish has real pre- and post-execution events, so it reports the full marker
 * set without the guards bash's DEBUG trap needs.
 */

function script(minimalPrompt: boolean): string {
  return `# Vorn shell integration for fish.

function __vorn_precmd --on-event fish_prompt
${
  minimalPrompt
    ? `    # The row the boundary rule is drawn on, skipped before the first prompt
    # so a session does not open on an empty line.
    if set -q __vorn_seen_prompt
        printf '\\n'
    end
    set -g __vorn_seen_prompt 1
`
    : ''
}    printf '\\033]5522;cwd;%s\\007' "$PWD"
    printf '\\033]133;A\\007'
end

function __vorn_preexec --on-event fish_preexec
    printf '\\033]5522;cmd;%s\\007' (printf '%s' -- $argv[1] | base64 | tr -d '\\n')
    printf '\\033]133;C\\007'
end

function __vorn_postexec --on-event fish_postexec
    # Must be the first statement, or it reports the status of whatever this
    # function did rather than the command's.
    set -l __vorn_status $status
${minimalPrompt ? "    printf '\\n'\n" : ''}    printf '\\033]133;D;%s\\007' $__vorn_status
end
${
  minimalPrompt
    ? `
# conf.d is read before the user's config.fish, so a prompt defined here would
# simply be replaced by theirs. Redefining from the prompt event instead runs
# after their config has been read; the very first prompt may still be their
# own, and every one after it is ours.
function __vorn_minimal_prompt --on-event fish_prompt
    functions --erase __vorn_minimal_prompt
    function fish_prompt
    end
    function fish_right_prompt
    end
end`
    : ''
}
`
}

export function fishSetup(opts: ShellOptions, existingDataDirs: string): ShellSetup {
  const dir = writeShimDir('fish', {
    'fish/vendor_conf.d/vorn.fish': script(opts.minimalPrompt)
  })
  return {
    env: {
      // Prepended, and the platform default appended when the user has none —
      // replacing XDG_DATA_DIRS outright would hide every other vendor's
      // completions and functions.
      XDG_DATA_DIRS: existingDataDirs
        ? `${dir}:${existingDataDirs}`
        : `${dir}:/usr/local/share:/usr/share`,
      VORN_MINIMAL_PROMPT: opts.minimalPrompt ? '1' : '0'
    },
    args: null
  }
}
