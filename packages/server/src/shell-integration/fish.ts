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
 * fish 4.0 and later mark prompts with OSC 133 on their own, and emit the
 * command line inside their C marker. Emitting our own alongside them
 * double-reports every boundary, which leaves the renderer with two prompts and
 * two finishes per command and no usable blocks at all. So on fish 4 we add
 * only the working directory, which its markers do not carry, and let fish do
 * the rest. Older fish, and anyone who has turned the feature off with
 * `no-mark-prompt`, gets the full set from us.
 */

function script(minimalPrompt: boolean): string {
  const gap = minimalPrompt ? "    printf '\\n'\n" : ''
  return `# Vorn shell integration for fish.

# fish 4 marks prompts itself; doing it again would report every boundary twice.
set -g __vorn_own_marks 1
if test (string split '.' -- $version)[1] -ge 4
    and not contains -- no-mark-prompt $fish_features
    set -g __vorn_own_marks 0
end

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
    if test $__vorn_own_marks -eq 1
        printf '\\033]133;A\\007'
    end
end

function __vorn_preexec --on-event fish_preexec
    test $__vorn_own_marks -eq 1; or return
    # No -- before the format: fish's printf has no end-of-options marker and
    # would print it as literal text, prefixing every captured command with it.
    printf '\\033]5522;cmd;%s\\007' (printf '%s' $argv[1] | base64 | tr -d '\\n')
    printf '\\033]133;C\\007'
end

function __vorn_postexec --on-event fish_postexec
    # Must be the first statement, or it reports the status of whatever this
    # function did rather than the command's.
    set -l __vorn_status $status
${gap}    if test $__vorn_own_marks -eq 1
        printf '\\033]133;D;%s\\007' $__vorn_status
    end
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
