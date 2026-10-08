#!/bin/sh
# Builds the Apple app: the Rust static library (grid client + GPU renderer)
# for each platform, the Xcode project from project.yml, then the macOS app
# and/or the iOS simulator app, one build at a time.
#   ./build.sh [mac|ios|all]   (default mac)
# Products: apple/build/VornSpikeApple.app, apple/build/VornSpikeIOS.app
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
what=${1:-mac}
cd "$here"
xcodegen generate --quiet
mkdir -p build
if [ "$what" = mac ] || [ "$what" = all ]; then
  (cd "$root" && cargo build --release -j 3 -p vorn-spike-ffi --target aarch64-apple-darwin)
  xcodebuild -quiet -project VornSpikeApple.xcodeproj -scheme VornSpikeMac -configuration Release \
    -derivedDataPath build/dd -jobs 3 build
  rm -rf build/VornSpikeApple.app
  cp -R build/dd/Build/Products/Release/VornSpikeApple.app build/
fi
if [ "$what" = ios ] || [ "$what" = all ]; then
  (cd "$root" && IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build --release -j 3 -p vorn-spike-ffi --target aarch64-apple-ios-sim)
  xcodebuild -quiet -project VornSpikeApple.xcodeproj -scheme VornSpikeIOS -configuration Release \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath build/dd -jobs 3 build
  rm -rf build/VornSpikeIOS.app
  cp -R build/dd/Build/Products/Release-iphonesimulator/VornSpikeIOS.app build/
fi
