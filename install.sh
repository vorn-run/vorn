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

    echo "${APP_NAME} ${VERSION} installed to /Applications/${APP_NAME}.app"
    ;;

  Linux)
    ARTIFACT="${APP_NAME}-${VERSION_NUM}.AppImage"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARTIFACT}"
    INSTALL_DIR="${HOME}/.local/bin"

    mkdir -p "$INSTALL_DIR"

    echo "Downloading ${ARTIFACT}..."
    curl -fSL --progress-bar -o "${INSTALL_DIR}/vorn" "$URL"
    chmod +x "${INSTALL_DIR}/vorn"

    if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
      echo ""
      echo "Add ${INSTALL_DIR} to your PATH:"
      echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      echo ""
      echo "Add this to your ~/.bashrc or ~/.zshrc to make it permanent."
    fi

    echo "${APP_NAME} ${VERSION} installed to ${INSTALL_DIR}/vorn"
    ;;

  *)
    echo "Error: Unsupported OS '${OS}'."
    echo "For Windows, use: irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
    exit 1
    ;;
esac

echo "Done!"
