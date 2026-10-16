#!/bin/sh
# Собрать мост clod-bridge для macOS как universal-бинарник (Apple Silicon +
# Intel) в src-tauri/resources/clod-bridge — его кладёт в .app бандлер Tauri
# (tauri.macos.conf.json → bundle.resources).
#
# Запуск: npm run bridge:build:mac (на Mac или в CI на macOS-раннере).
set -eu
cd "$(dirname "$0")/.."

TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
if command -v rustup >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    rustup target add $TARGETS
fi

for t in $TARGETS; do
    cargo build --release --manifest-path bridge/Cargo.toml --target "$t"
done

out=src-tauri/resources/clod-bridge
rm -f "$out"
lipo -create -output "$out" \
    bridge/target/aarch64-apple-darwin/release/clod-bridge \
    bridge/target/x86_64-apple-darwin/release/clod-bridge
# Apple Silicon не запускает неподписанный код — ad-hoc подпись после lipo.
codesign --force --sign - "$out"
lipo -info "$out"
"$out" --version
