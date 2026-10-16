#!/bin/sh
# Builds both apps; PROFILE=<.mobileprovision> signs them and a device id installs them.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
core="$here/../../packages/core"
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
(cd "$core" && IPHONEOS_DEPLOYMENT_TARGET=26.0 nice -n 10 cargo build --profile release -j 3 -p vorn-phone --target aarch64-apple-ios)
cd "$here"
xcodegen generate --quiet
for t in Drawn Native; do
  nice -n 10 xcodebuild -quiet -project VornPhone.xcodeproj -scheme "$t" -configuration Release \
    -destination generic/platform=iOS -derivedDataPath build VORN_HOST="${VORN_HOST:-}" build
done
if [ -n "${PROFILE:-}" ]; then
  security cms -D -i "$PROFILE" > build/profile.plist
  /usr/libexec/PlistBuddy -x -c "Print :Entitlements" build/profile.plist > build/entitlements.plist
  mine=$(security find-identity -v -p codesigning)
  cert=""
  for sha in $(python3 -c 'import plistlib,hashlib,sys;[print(hashlib.sha1(c).hexdigest().upper()) for c in plistlib.load(open(sys.argv[1],"rb"))["DeveloperCertificates"]]' build/profile.plist); do
    case "$mine" in *"$sha"*) cert=$sha; break ;; esac
  done
  [ -n "$cert" ] || { echo "no identity in the keychain matches the profile" >&2; exit 1; }
  for app in build/Build/Products/Release-iphoneos/*.app; do
    cp "$PROFILE" "$app/embedded.mobileprovision"
    codesign -f -s "$cert" --keychain "$HOME/Library/Keychains/login.keychain-db" --entitlements build/entitlements.plist "$app"
  done
fi
for app in build/Build/Products/Release-iphoneos/*.app; do
  echo "$app $(du -sh "$app" | cut -f1) (binary $(du -h "$app/$(basename "$app" .app)" | cut -f1))"
done
if [ -n "${1:-}" ]; then
  for app in build/Build/Products/Release-iphoneos/*.app; do
    DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer xcrun devicectl device install app --device "$1" "$app"
  done
fi
