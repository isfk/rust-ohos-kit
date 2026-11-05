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
# The database is not part of the core — a C library lives in here — but it is
# still testable on a laptop: that is what keeping it in its own crate buys.
cargo test -p rustkit-store
cargo test -p rustkit-bridge --lib

echo "== colour ramps =="
# Every token the light theme declares, the dark one has to declare too: a name
# that exists in only one of them resolves to nothing on the other theme, which
# shows up as a black or transparent panel on a device — never on a Mac.
light=app/entry/src/main/resources/base/element/color.json
dark=app/entry/src/main/resources/dark/element/color.json
if [ -f "$light" ] && [ -f "$dark" ]; then
	names() { grep -o '"name": *"[^"]*"' "$1" | sed 's/.*"\(.*\)"/\1/' | sort; }
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT INT TERM
	names "$light" >"$tmp/light"
	names "$dark" >"$tmp/dark"
	if ! cmp -s "$tmp/light" "$tmp/dark"; then
		echo "light/dark colour token sets differ" >&2
		cmp "$tmp/light" "$tmp/dark" >&2 || true
		exit 1
	fi
	echo "both ramps declare $(wc -l <"$tmp/light" | tr -d ' ') matching tokens"
else
	echo "no app colours to compare (app not carried over)" >&2
fi

echo "== shared conclusions page =="
# docs/go-vs-rust.md exists in both repositories, on purpose: each one has to read
# correctly on its own. Two copies drift, so this compares them when the sibling
# checkout happens to be next to this one — and says so instead of pretending to
# have checked when it is not.
sibling=../go-ohos-kit/docs/go-vs-rust.md
mine=docs/go-vs-rust.md
if [ -f "$sibling" ]; then
	if ! cmp -s "$mine" "$sibling"; then
		echo "docs/go-vs-rust.md differs from $sibling — change both, or neither" >&2
		diff "$mine" "$sibling" | head -20 >&2
		exit 1
	fi
	echo "identical to the sibling's copy"
else
	echo "no sibling checkout: skipped"
fi

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
