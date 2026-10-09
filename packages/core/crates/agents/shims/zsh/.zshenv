# Vorn shell integration bootstrap — loads your own zsh files unchanged.
VORN_SHIM_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="${VORN_USER_ZDOTDIR:-$HOME}"
[[ -f "$ZDOTDIR/.zshenv" ]] && builtin source "$ZDOTDIR/.zshenv"
VORN_USER_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$VORN_SHIM_ZDOTDIR"
