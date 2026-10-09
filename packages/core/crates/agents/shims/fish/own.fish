# Vorn shell integration for fish.

# fish 4 marks prompts itself; doing it again would report every boundary twice.
set -g __vorn_own_marks 1
if test (string split '.' -- $version)[1] -ge 4
    and not contains -- no-mark-prompt $fish_features
    set -g __vorn_own_marks 0
end

function __vorn_precmd --on-event fish_prompt
    printf '\033]5522;cwd;%s\007' "$PWD"
    if test $__vorn_own_marks -eq 1
        printf '\033]133;A\007'
    end
end

function __vorn_preexec --on-event fish_preexec
    test $__vorn_own_marks -eq 1; or return
    # No -- before the format: fish's printf has no end-of-options marker and
    # would print it as literal text, prefixing every captured command with it.
    printf '\033]5522;cmd;%s\007' (printf '%s' $argv[1] | base64 | tr -d '\n')
    printf '\033]133;C\007'
end

function __vorn_postexec --on-event fish_postexec
    # Must be the first statement, or it reports the status of whatever this
    # function did rather than the command's.
    set -l __vorn_status $status
    if test $__vorn_own_marks -eq 1
        printf '\033]133;D;%s\007' $__vorn_status
    end
end

