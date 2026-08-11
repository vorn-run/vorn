#!/bin/bash
# Syncs all workspace package versions with the root package.json version.
# Run automatically via pre-commit hook when package.json version changes.

ROOT_DIR="$(git rev-parse --show-toplevel)"
ROOT_VERSION=$(node -p "require('$ROOT_DIR/package.json').version")

PACKAGES=(
  "packages/web"
  "packages/desktop"
  "packages/server"
  "packages/shared"
  "packages/mcp"
  "packages/connector-sdk"
)

OUT_OF_SYNC=0

for pkg in "${PACKAGES[@]}"; do
  PKG_FILE="$ROOT_DIR/$pkg/package.json"
  if [ -f "$PKG_FILE" ]; then
    PKG_VERSION=$(node -p "require('$PKG_FILE').version")
    if [ "$PKG_VERSION" != "$ROOT_VERSION" ]; then
      echo "Syncing $pkg: $PKG_VERSION → $ROOT_VERSION"
      node -e "
        const fs = require('fs');
        const pkg = JSON.parse(fs.readFileSync('$PKG_FILE', 'utf8'));
        pkg.version = '$ROOT_VERSION';
        fs.writeFileSync('$PKG_FILE', JSON.stringify(pkg, null, 2) + '\n');
      "
      git add "$PKG_FILE"
      OUT_OF_SYNC=1
    fi
  fi
done

if [ "$OUT_OF_SYNC" -eq 1 ]; then
  echo "✓ All package versions synced to $ROOT_VERSION"
fi
