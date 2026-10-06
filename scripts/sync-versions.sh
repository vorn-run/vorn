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
  "packages/core"
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

# The crate reports its version to the server, so it follows the app's too.
CARGO_FILE="$ROOT_DIR/packages/core/Cargo.toml"
if [ -f "$CARGO_FILE" ]; then
  CARGO_CHANGED=$(node -e "
    const fs = require('fs');
    const toml = fs.readFileSync('$CARGO_FILE', 'utf8');
    const next = toml.replace(/^version = \".*\"$/m, 'version = \"$ROOT_VERSION\"');
    if (next !== toml) { fs.writeFileSync('$CARGO_FILE', next); console.log('yes'); }
  ")
  if [ -n "$CARGO_CHANGED" ]; then
    echo "Syncing packages/core/Cargo.toml → $ROOT_VERSION"
    git add "$CARGO_FILE"
    OUT_OF_SYNC=1
  fi
fi

# The vorn command prints its crate's version, so it follows the app's too.
CLI_FILE="$ROOT_DIR/packages/core/crates/cli/Cargo.toml"
if [ -f "$CLI_FILE" ]; then
  CLI_CHANGED=$(node -e "
    const fs = require('fs');
    const toml = fs.readFileSync('$CLI_FILE', 'utf8');
    const next = toml.replace(/^version = \".*\"$/m, 'version = \"$ROOT_VERSION\"');
    if (next !== toml) { fs.writeFileSync('$CLI_FILE', next); console.log('yes'); }
  ")
  if [ -n "$CLI_CHANGED" ]; then
    echo "Syncing packages/core/crates/cli/Cargo.toml → $ROOT_VERSION"
    git add "$CLI_FILE"
    OUT_OF_SYNC=1
  fi
fi

# Cargo.lock records the crate's own version too, and CI builds with --locked.
# Checked on its own: the lock can be stale while Cargo.toml is already right.
LOCK_FILE="$ROOT_DIR/packages/core/Cargo.lock"
if [ -f "$LOCK_FILE" ]; then
  LOCK_CHANGED=$(node -e "
    const fs = require('fs');
    const lock = fs.readFileSync('$LOCK_FILE', 'utf8');
    const next = lock
      .replace(/(name = \"vorn-core\"\nversion = )\".*\"/, '\$1\"$ROOT_VERSION\"')
      .replace(/(name = \"vorn-cli\"\nversion = )\".*\"/, '\$1\"$ROOT_VERSION\"');
    if (next !== lock) { fs.writeFileSync('$LOCK_FILE', next); console.log('yes'); }
  ")
  if [ -n "$LOCK_CHANGED" ]; then
    echo "Syncing packages/core/Cargo.lock → $ROOT_VERSION"
    git add "$LOCK_FILE"
    OUT_OF_SYNC=1
  fi
fi

if [ "$OUT_OF_SYNC" -eq 1 ]; then
  echo "✓ All package versions synced to $ROOT_VERSION"
fi
