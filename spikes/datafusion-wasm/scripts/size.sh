#!/usr/bin/env bash
# Build one variant with wasm-pack and print raw / wasm-opt -Oz / gzip -9 / brotli sizes.
# Usage: scripts/size.sh <label> [cargo feature args...]
set -euo pipefail
cd "$(dirname "$0")/.."
label=$1; shift
out=target/size/$label
rm -rf "$out"
wasm-pack build --release --target web --out-dir "$out" "$@" >/dev/null 2>&1
raw=$out/datafusion_wasm_spike_bg.wasm
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-reference-types --enable-multivalue \
  "$raw" -o "$out/opt.wasm"
gzip -9 -k -c "$out/opt.wasm" > "$out/opt.wasm.gz"
size() { stat -f %z "$1"; }
br=""
if command -v brotli >/dev/null; then brotli -q 11 -k -f "$out/opt.wasm" -o "$out/opt.wasm.br"; br=$(size "$out/opt.wasm.br"); fi
printf '%-12s raw=%s opt=%s gz=%s br=%s\n' "$label" "$(size "$raw")" "$(size "$out/opt.wasm")" "$(size "$out/opt.wasm.gz")" "$br"
