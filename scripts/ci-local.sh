#!/usr/bin/env bash
set -e

# Mirrors the CI pipeline. Run before pushing to catch failures locally.
# Requires: diff-cover (pipx install diff_cover) for the coverage gate.

cd "$(dirname "$0")/.."

step() {
  printf "\n\033[1;36m▶ %s\033[0m\n" "$1"
}

step "Typecheck"
yarn typecheck

step "Lockfile has no private registry references"
yarn lint:lockfile

step "Lint"
yarn lint

step "Format check"
yarn format:check

step "Test with coverage"
yarn test:coverage

if command -v diff-cover >/dev/null 2>&1; then
  step "Enforce 80% patch coverage vs origin/main"
  if ! git rev-parse --verify origin/main >/dev/null 2>&1; then
    echo "  warning: origin/main not found locally — fetching"
    git fetch origin main --quiet
  fi
  diff-cover coverage/lcov.info --compare-branch=origin/main --fail-under=80
else
  printf "\n\033[1;33m⚠  diff-cover not installed — skipping patch coverage gate.\033[0m\n"
  printf "   Install once with: \033[1mpipx install diff_cover\033[0m\n"
fi

printf "\n\033[1;32m✓ All CI checks passed locally\033[0m\n"
