#!/bin/sh
# Everything a machine without a phone can prove.
#
#   scripts/check.sh             fmt, clippy, tests, then the device build
#   scripts/check.sh --no-build  skip the cross build (a machine with no SDK)
set -eu

cd "$(dirname "$0")/.."
build=yes
if [ "${1:-}" = "--no-build" ]; then
	build=no
fi

echo "== cargo fmt =="
cargo fmt --all -- --check

echo "== cargo clippy =="
cargo clippy --all-targets --all-features -- -D warnings

echo "== cargo test =="
# The core only. The bridge is a `cdylib` that exists to be dlopen'd by
# HarmonyOS: its own tests are the three that do not need a phone attached.
cargo test -p rustkit-core
cargo test -p rustkit-bridge --lib

if [ "$build" = yes ]; then
	echo "== cross build: aarch64-unknown-linux-ohos =="
	so=$(./platform/ohos/build.sh 2>/dev/null | tail -1)
	toolchain=${DEVECO_SDK_HOME:-/Users/shen/command-line-tools/sdk}/default/openharmony/native/llvm/bin

	# The Go version needed four runtime patches, and one of them existed to keep
	# a TLS relocation out of the artifact. Rust has no such relocation to keep
	# out — this asserts that, rather than assuming it.
	tprel=$("$toolchain/llvm-readelf" -r "$so" | grep -c TPREL || true)
	if [ "$tprel" != 0 ]; then
		echo "TLS_TPREL64 relocations: $tprel (want 0)" >&2
		exit 1
	fi

	# A module with no registration is a module ArkTS cannot import, and the
	# build says nothing about it: the C glue is only pulled in because
	# build.rs hands the linker its object file directly.
	undefined=$("$toolchain/llvm-nm" -D -u "$so")
	case $undefined in
	*napi_module_register*) ;;
	*)
		echo "the .so does not reference napi_module_register: the C glue was dropped" >&2
		exit 1
		;;
	esac

	# The libraries the module must name. An undefined `napi_module_register` is
	# accepted by the linker and rejected by the device, at dlopen, after the HAP
	# has been installed: "symbol not found". The Go version asserts the same list.
	needed=$("$toolchain/llvm-readelf" -d "$so" | grep NEEDED)
	case $needed in
	*libace_napi.z.so*) ;;
	*)
		echo "the .so does not link libace_napi.z.so: it will fail to load on device" >&2
		exit 1
		;;
	esac
	case $needed in
	*libhilog_ndk.z.so*) ;;
	*)
		echo "the .so does not link libhilog_ndk.z.so: hilog will not resolve" >&2
		exit 1
		;;
	esac

	exports=$("$toolchain/llvm-nm" -D --defined-only "$so" | grep -c ' rustkit_')
	echo "rustkit: $(basename "$so") — $(du -h "$so" | cut -f1), $exports exports, $tprel TPRELs"
fi

echo "== done =="
