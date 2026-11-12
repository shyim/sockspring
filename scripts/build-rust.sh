#!/bin/sh
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
export ANDROID_NDK_HOME="$1/ndk/$2"
output=$3
command -v cargo-ndk >/dev/null || { echo 'Install cargo-ndk: cargo install cargo-ndk --locked' >&2; exit 1; }
[ -d "$ANDROID_NDK_HOME" ] || { echo "Install the NDK: sdkmanager 'ndk;$2'" >&2; exit 1; }
# Native libraries must support Android devices with 16 KiB pages.
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"
cd native
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86 -t x86_64 --platform 24 -o "$output" build --release --lib --locked
