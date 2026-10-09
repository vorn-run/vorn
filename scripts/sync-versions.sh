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

# Every crate that reports a version to the app inherits the workspace's.
CARGO_FILE="$ROOT_DIR/packages/core/Cargo.toml"
if [ -f "$CARGO_FILE" ]; then
  CARGO_CHANGED=$(node -e "
    const fs = require('fs');
    const toml = fs.readFileSync('$CARGO_FILE', 'utf8');
    const next = toml.replace(/(\\[workspace\\.package\\]\r?\nversion = )\".*\"/, '\$1\"$ROOT_VERSION\"');
    if (next !== toml) { fs.writeFileSync('$CARGO_FILE', next); console.log('yes'); }
  ")
  if [ -n "$CARGO_CHANGED" ]; then
    echo "Syncing packages/core/Cargo.toml → $ROOT_VERSION"
    git add "$CARGO_FILE"
    OUT_OF_SYNC=1
  fi
fi

# Cargo.lock records each inheriting crate's version too, and CI builds with --locked.
# Checked on its own: the lock can be stale while Cargo.toml is already right.
LOCK_FILE="$ROOT_DIR/packages/core/Cargo.lock"
if [ -f "$LOCK_FILE" ]; then
  LOCK_CHANGED=$(node -e "
    const fs = require('fs');
    const lock = fs.readFileSync('$LOCK_FILE', 'utf8');
    let next = lock;
    const path = require('path');
    const core = path.dirname('$LOCK_FILE');
    // Every workspace crate that inherits the workspace version, found from its manifest.
    const manifests = [path.join(core, 'Cargo.toml')];
    for (const dir of ['crates', 'bench']) {
      const base = path.join(core, dir);
      if (!fs.existsSync(base)) continue;
      for (const d of fs.readdirSync(base)) manifests.push(path.join(base, d, 'Cargo.toml'));
    }
    const names = manifests
      .filter((m) => fs.existsSync(m))
      .map((m) => fs.readFileSync(m, 'utf8'))
      .filter((t) => /^version\\.workspace\\s*=\\s*true/m.test(t) || /^version\\s*=\\s*\\{\\s*workspace/m.test(t))
      .map((t) => (t.match(/^name\\s*=\\s*\"([^\"]+)\"/m) || [])[1])
      .filter(Boolean);
    for (const name of names) {
      next = next.replace(new RegExp('(name = \"' + name + '\"\\r?\\nversion = )\".*\"'), '\$1\"$ROOT_VERSION\"');
    }
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
