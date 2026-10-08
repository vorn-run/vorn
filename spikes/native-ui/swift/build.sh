#!/bin/sh
# Builds swift/build/VornSpikeSwift.app: the Rust grid client as a static
# library, this file compiled with swiftc, a hand-made bundle.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
(cd "$root" && cargo build --release -j 3 -p vorn-spike-ffi)
app="$here/build/VornSpikeSwift.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
swiftc -O -module-name VornSpikeSwift \
  -import-objc-header "$root/ffi/include/vorn_spike.h" \
  "$here/main.swift" \
  "$root/target/release/libvorn_spike_ffi.a" \
  -framework AppKit -framework SwiftUI -framework CoreText -framework QuartzCore \
  -o "$app/Contents/MacOS/VornSpikeSwift"
strip -x "$app/Contents/MacOS/VornSpikeSwift"
cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>VornSpikeSwift</string>
  <key>CFBundleIdentifier</key><string>run.vorn.spike.swift</string>
  <key>CFBundleName</key><string>VornSpikeSwift</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
codesign -s - --force "$app" >/dev/null 2>&1 || true
echo "$app"
