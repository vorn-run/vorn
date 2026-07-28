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

step "Completion index is up to date"
# The index is committed so builds need neither the corpus nor the network.
# Regenerating must be a no-op; if it is not, someone edited it by hand.
# Output is not silenced: without the corpus the generator skips, and that
# needs to be visible rather than looking like a passing check.
yarn gen:completions
git diff --exit-code src/renderer/lib/completion-index
# The generator cannot run without the corpus, so verify the committed index
# was built from the committed allowlist. This is what actually catches an
# allowlist edited without regenerating.
node scripts/check-completion-allowlist.mjs

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

step "Verify server CJS bundle"
yarn workspace @vornrun/server build
if grep -q 'import_meta' packages/server/dist/index.cjs; then
  echo "ERROR: import_meta found in CJS bundle — will crash at runtime"
  exit 1
fi

printf "\n\033[1;32m✓ All CI checks passed locally\033[0m\n"
