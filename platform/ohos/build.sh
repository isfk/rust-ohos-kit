#!/bin/sh
# Builds the Rust core into librustcore.so for the device, and puts it where the
# app expects it.
#
# Cross-compiling needs no patched toolchain, which is the claim this repository
# exists to test: the OHOS NDK ships a clang wrapper named exactly what cargo
# asks for (`aarch64-unknown-linux-ohos-clang`), and it already carries
# `-target aarch64-linux-ohos --sysroot=<ndk>/native/sysroot`.
#
#   platform/ohos/build.sh [app project dir]
set -eu

sdk=${DEVECO_SDK_HOME:-/Users/shen/command-line-tools/sdk}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
project=${1:-$root/app}
toolchain=$sdk/default/openharmony/native/llvm/bin
target=aarch64-unknown-linux-ohos

PATH="$toolchain:$PATH"
export PATH

if [ ! -x "$toolchain/aarch64-unknown-linux-ohos-clang" ]; then
	echo "rustkit: no OHOS toolchain at $toolchain" >&2
	echo "rustkit: set DEVECO_SDK_HOME to a DevEco SDK (default /Users/shen/command-line-tools/sdk)" >&2
	exit 1
fi

cd "$root"
echo "rustkit: building the core for $target" >&2
cargo build -p rustkit-bridge --target "$target" --release

so=$root/target/$target/release/librustcore.so
"$toolchain/llvm-strip" --strip-debug "$so"

if [ -d "$project/entry/src" ]; then
	dest=$project/entry/libs/arm64-v8a
	mkdir -p "$dest"
	cp "$so" "$dest/librustcore.so"
	echo "rustkit: $dest/librustcore.so" >&2
else
	echo "rustkit: no app at $project — carry the ArkTS shell over (see CONTEXT.md)" >&2
fi

echo "$so"
