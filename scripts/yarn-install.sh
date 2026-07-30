#!/usr/bin/env bash
# Installs dependencies on machines where public npm is firewalled.
#
# Yarn Berry ignores ~/.npmrc, so a mirror configured for npm is invisible to
# it and `yarn install` fails with an opaque ENOTCONN/RequestError. This reads
# the mirror from the local npm config (never committed — it is machine
# specific) and hands it to Yarn, then strips the private archive URLs Yarn
# pins into yarn.lock when the mirror serves tarballs from another host.
#
# Usage: yarn deps:install [extra yarn install args]

set -e

cd "$(dirname "$0")/.."

DEFAULT_REGISTRY="https://registry.npmjs.org"
registry="$(npm config get registry 2>/dev/null || true)"
registry="${registry%/}"

if [ -n "$registry" ] && [ "$registry" != "undefined" ] && [ "$registry" != "$DEFAULT_REGISTRY" ]; then
  printf "\033[1;36m▶ Using npm registry from local npm config: %s\033[0m\n" "$registry"
  export YARN_NPM_REGISTRY_SERVER="$registry"
else
  printf "\033[1;36m▶ Using default public npm registry\033[0m\n"
fi

# Keep going on failure so the lockfile is still sanitised: Yarn writes the
# updated lockfile during resolution, then may fail later in the fetch step if
# the mirror has not ingested a newly published version yet.
install_status=0
yarn install "$@" || install_status=$?

node scripts/check-lockfile.mjs --fix

if [ "$install_status" -ne 0 ]; then
  printf "\n\033[1;33m⚠  yarn install exited with status %s.\033[0m\n" "$install_status"
  printf "   If this is a 404 on a recently published version, the mirror has not\n"
  printf "   ingested it yet. CI resolves against public npm and is unaffected.\n"
  exit "$install_status"
fi

printf "\n\033[1;32m✓ Dependencies installed and yarn.lock validated\033[0m\n"
