#!/bin/sh
set -e

REPO="vorn-run/vorn"
APP_NAME="Vorn"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

get_latest_version() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | sed 's/.*"tag_name": *"//;s/".*//'
}

# -F: a directory holding a dot -- ~/.local/bin, say -- is not a regex.
on_path() {
  echo ":$PATH:" | grep -qF ":$1:"
}

# Where the `vorn` command goes: a writable directory the shell already searches,
# or, failing that, the per-user one, which is created and printed with a hint.
choose_bin_dir() {
  user_bin="${HOME}/.local/bin"
  for dir in "/usr/local/bin" "$user_bin"; do
    if [ -w "$dir" ] && on_path "$dir"; then
      echo "$dir"
      return
    fi
  done
  for dir in "/usr/local/bin" "$user_bin"; do
    if [ -w "$dir" ]; then
      echo "$dir"
      return
    fi
  done
  echo "$user_bin"
}

path_hint() {
  if ! on_path "$1"; then
    echo ""
    echo "Add ${1} to your PATH:"
    echo "  export PATH=\"${1}:\$PATH\""
    echo ""
    echo "Add this to your ~/.bashrc or ~/.zshrc to make it permanent."
  fi
}

VERSION="${VORN_VERSION:-$(get_latest_version)}"

if [ -z "$VERSION" ]; then
  echo "Error: Could not determine latest version."
  echo "Set VORN_VERSION=vX.Y.Z to install a specific version."
  exit 1
fi

VERSION_NUM="${VERSION#v}"

echo "Installing ${APP_NAME} ${VERSION}..."

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64) DMG_ARCH="arm64" ;;
      *)     DMG_ARCH="x64" ;;
    esac
    ARTIFACT="${APP_NAME}-${VERSION_NUM}-${DMG_ARCH}.dmg"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARTIFACT}"
    TMPDIR_INSTALL="$(mktemp -d)"

    echo "Downloading ${ARTIFACT}..."
    curl -fSL --progress-bar -o "${TMPDIR_INSTALL}/${ARTIFACT}" "$URL"

    echo "Mounting DMG..."
    MOUNT_POINT="$(hdiutil attach "${TMPDIR_INSTALL}/${ARTIFACT}" -nobrowse | tail -1 | sed 's/.*	//')"

    if [ -d "/Applications/${APP_NAME}.app" ]; then
      echo "Removing previous installation..."
      rm -rf "/Applications/${APP_NAME}.app"
    fi

    echo "Installing to /Applications..."
    cp -R "${MOUNT_POINT}/${APP_NAME}.app" /Applications/

    echo "Cleaning up..."
    hdiutil detach "$MOUNT_POINT" -quiet
    rm -rf "$TMPDIR_INSTALL"

    BIN_DIR="$(choose_bin_dir)"
    mkdir -p "$BIN_DIR"

    # The command runs the app's own Node — the same trick the app uses to start
    # its server — so nothing else has to be installed and the native modules
    # inside the bundle resolve.
    cat > "${BIN_DIR}/vorn" <<'SHIM'
#!/bin/sh
APP="/Applications/Vorn.app"
RESOURCES="${APP}/Contents/Resources"

# No command: open the app, which is what typing `vorn` should mean.
if [ "$#" -eq 0 ]; then
  exec open -a "$APP"
fi

ELECTRON_RUN_AS_NODE=1
VORN_NATIVE_MODULES_PATH="${RESOURCES}/app.asar.unpacked/node_modules"
NODE_PATH="${RESOURCES}/app.asar/node_modules:${VORN_NATIVE_MODULES_PATH}"
export ELECTRON_RUN_AS_NODE VORN_NATIVE_MODULES_PATH NODE_PATH

exec "${APP}/Contents/MacOS/Vorn" "${RESOURCES}/server/cli.cjs" "$@"
SHIM
    chmod +x "${BIN_DIR}/vorn"

    echo "${APP_NAME} ${VERSION} installed to /Applications/${APP_NAME}.app"
    echo "The vorn command is at ${BIN_DIR}/vorn"
    path_hint "$BIN_DIR"
    ;;

  Linux)
    ARTIFACT="${APP_NAME}-${VERSION_NUM}.AppImage"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARTIFACT}"
    BIN_DIR="${HOME}/.local/bin"
    LIB_DIR="${HOME}/.local/lib/vorn"

    mkdir -p "$BIN_DIR" "$LIB_DIR"

    # Earlier installs put the AppImage itself here. The new one is downloaded
    # to LIB_DIR below, so the old file is removed rather than moved -- said
    # plainly, because a message about moving would send you looking for it.
    if [ -f "${BIN_DIR}/vorn" ] && [ ! -f "${LIB_DIR}/${APP_NAME}.AppImage" ]; then
      echo "Removing the old AppImage at ${BIN_DIR}/vorn; the app now lives in ${LIB_DIR}."
      rm -f "${BIN_DIR}/vorn"
    fi

    echo "Downloading ${ARTIFACT}..."
    curl -fSL --progress-bar -o "${LIB_DIR}/${APP_NAME}.AppImage" "$URL"
    chmod +x "${LIB_DIR}/${APP_NAME}.AppImage"

    cat > "${BIN_DIR}/vorn" <<'SHIM'
#!/bin/sh
APPIMAGE="${HOME}/.local/lib/vorn/Vorn.AppImage"

# No command: open the app, which is what typing `vorn` should mean.
if [ "$#" -eq 0 ]; then
  exec "$APPIMAGE"
fi

# The CLI lives inside the AppImage, whose mount point is only known once it is
# running — so the entry point is resolved from APPDIR in there, and spliced
# into argv where a normally-invoked script would have been.
BOOTSTRAP='var entry = process.env.APPDIR + "/resources/server/cli.cjs"; process.argv.splice(1, 0, entry); require(entry)'

ELECTRON_RUN_AS_NODE=1 exec "$APPIMAGE" -e "$BOOTSTRAP" "$@"
SHIM
    chmod +x "${BIN_DIR}/vorn"

    echo "${APP_NAME} ${VERSION} installed to ${LIB_DIR}/${APP_NAME}.AppImage"
    echo "The vorn command is at ${BIN_DIR}/vorn"
    path_hint "$BIN_DIR"
    ;;

  *)
    echo "Error: Unsupported OS '${OS}'."
    echo "For Windows, use: irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
    exit 1
    ;;
esac

echo "Done!"
