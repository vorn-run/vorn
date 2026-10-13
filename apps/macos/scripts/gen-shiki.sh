#!/bin/sh
# Bundles Shiki (vitesse-dark, the grammars shiki.ts maps to) for JavaScriptCore: Sources/VornUI/Resources/shiki.js.
set -eu
root="$(cd "$(dirname "$0")/../../.." && pwd)"
NODE_PATH="$root/node_modules" "$root/node_modules/.bin/esbuild" "$(dirname "$0")/shiki/entry.mjs" \
  --bundle --format=iife --minify --target=safari17 --legal-comments=none \
  --outfile="$(dirname "$0")/../Sources/VornUI/Resources/shiki.js"
