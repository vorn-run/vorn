#!/bin/sh
# Builds the web client vornd serves (`vornd --web <dir>`): the app compiled
# to WebAssembly, the loader page wasm-bindgen's glue needs, the fonts it
# draws with and brotli copies of each, into crates/vorn-web/dist (or $OUT).
#
# wasm-bindgen (at the version Cargo.lock pins) and wasm-opt are taken from
# $WASM_BINDGEN / $WASM_OPT or PATH, else fetched into target/web-tools.
# CARGO_FLAGS adds cargo arguments, e.g. "-j 3".
set -eu
core=$(cd "$(dirname "$0")/.." && pwd)
crate="$core/crates/vorn-web"
out=${OUT:-"$crate/dist"}
tools="$core/target/web-tools"
binaryen=version_133
target=wasm32-unknown-unknown

bindgen_version=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/"/,"",$3); print $3}' "$core/Cargo.lock")

fetch() {
  url=$1 dest=$2
  mkdir -p "$dest"
  curl -fsSL "$url" -o "$dest/tool.tar.gz"
  sum=$(curl -fsSL "$url.$3" | awk '{print $1}')
  got=$( (sha256sum "$dest/tool.tar.gz" 2>/dev/null || shasum -a 256 "$dest/tool.tar.gz") | awk '{print $1}')
  [ "$sum" = "$got" ] || { echo "checksum mismatch for $url" >&2; exit 1; }
  tar -xzf "$dest/tool.tar.gz" -C "$dest"
  rm "$dest/tool.tar.gz"
}

platform() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) echo "aarch64-apple-darwin arm64-macos" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin x86_64-macos" ;;
    Linux-x86_64) echo "x86_64-unknown-linux-musl x86_64-linux" ;;
    Linux-aarch64) echo "aarch64-unknown-linux-musl aarch64-linux" ;;
    *) echo "no prebuilt wasm tools for $(uname -sm); set WASM_BINDGEN and WASM_OPT" >&2; exit 1 ;;
  esac
}

bindgen=${WASM_BINDGEN:-$(command -v wasm-bindgen || true)}
if [ -n "$bindgen" ] && ! "$bindgen" --version | grep -q " $bindgen_version"; then
  bindgen=""
fi
if [ -z "$bindgen" ]; then
  set -- $(platform)
  dir="$tools/wasm-bindgen-$bindgen_version-$1"
  [ -x "$dir/wasm-bindgen" ] || fetch \
    "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$bindgen_version/wasm-bindgen-$bindgen_version-$1.tar.gz" \
    "$tools" sha256sum
  bindgen="$dir/wasm-bindgen"
fi
opt=${WASM_OPT:-$(command -v wasm-opt || true)}
if [ -z "$opt" ]; then
  set -- $(platform)
  opt="$tools/binaryen-$binaryen/bin/wasm-opt"
  [ -x "$opt" ] || fetch \
    "https://github.com/WebAssembly/binaryen/releases/download/$binaryen/binaryen-$binaryen-$2.tar.gz" \
    "$tools" sha256
fi

cd "$core"
rustup target add "$target" >/dev/null 2>&1 || true
# shellcheck disable=SC2086
cargo build --locked --profile wasm --target "$target" -p vorn-web ${CARGO_FLAGS:-}

rm -rf "$out"
mkdir -p "$out"
"$bindgen" --target web --no-typescript --out-dir "$out" "target/$target/wasm/vorn_web.wasm"
"$opt" -Oz --strip-debug --strip-producers \
  --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-reference-types --enable-multivalue \
  "$out/vorn_web_bg.wasm" -o "$out/vorn_web_bg.wasm"
cp -R "$crate/static/." "$out/"

cat > "$out/index.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content">
<meta name="theme-color" content="#0d0d0f">
<meta name="mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<link rel="manifest" href="manifest.webmanifest">
<link rel="icon" type="image/png" sizes="192x192" href="icon-192.png">
<link rel="apple-touch-icon" href="icon-192.png">
<link rel="modulepreload" href="vorn_web.js">
<link rel="preload" href="vorn_web_bg.wasm" as="fetch" type="application/wasm" crossorigin>
<title>Vorn</title>
<style>html,body{margin:0;height:100%;background:#0d0d0f;overflow:hidden;overscroll-behavior:none}</style>
</head>
<body>
<noscript>Vorn needs JavaScript and WebAssembly.</noscript>
<script type="module">import init from "./vorn_web.js"; init();</script>
</body>
</html>
HTML

cat > "$out/manifest.webmanifest" <<'JSON'
{
  "id": "/app/",
  "name": "Vorn",
  "short_name": "Vorn",
  "start_url": "/app/",
  "scope": "/app/",
  "display": "standalone",
  "orientation": "any",
  "background_color": "#0d0d0f",
  "theme_color": "#0d0d0f",
  "icons": [
    { "src": "icon-192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "icon-512.png", "sizes": "512x512", "type": "image/png" }
  ]
}
JSON

find "$out" -type f ! -name '*.png' ! -name '*.br' | while read -r f; do
  brotli -f -q 11 -o "$f.br" "$f"
done

wasm="$out/vorn_web_bg.wasm"
echo "wasm: $(wc -c < "$wasm" | tr -d ' ') bytes, $(wc -c < "$wasm.br" | tr -d ' ') brotli"
