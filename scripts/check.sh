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

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "== cargo fmt =="
cargo fmt --all -- --check

echo "== cargo clippy =="
# `--workspace` is not decoration: the root's `default-members` is core alone (a
# host build must not pull a phone in), and `cargo clippy` honours that — so
# without the flag this gate only ever lints rust/core, while store and bridge
# drift unlinted and the docs say they were checked.
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "== cargo test =="
# The core only. The bridge is a `cdylib` that exists to be dlopen'd by
# HarmonyOS: bridge tests exercise its host-safe FFI boundary without a phone.
cargo test -p rustkit-core
# The database is not part of the core — a C library lives in here — but it is
# still testable on a laptop: that is what keeping it in its own crate buys.
cargo test -p rustkit-store
cargo test -p rustkit-bridge --lib

echo "== scaffold =="
sh -n platform/ohos/new.sh
platform/ohos/new.sh --help >/dev/null 2>&1

echo "== colour ramps =="
# Every token the light theme declares, the dark one has to declare too: a name
# that exists in only one of them resolves to nothing on the other theme, which
# shows up as a black or transparent panel on a device — never on a Mac.
light=app/entry/src/main/resources/base/element/color.json
dark=app/entry/src/main/resources/dark/element/color.json
if [ -f "$light" ] && [ -f "$dark" ]; then
	names() { grep -o '"name": *"[^"]*"' "$1" | sed 's/.*"\(.*\)"/\1/' | sort; }
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

echo "== NAPI surface =="
# Two lists of the same names: what `Init()` registers, and what ArkTS is told
# exists. Registered but undeclared is a function nobody can type; declared but
# never registered is `undefined` at the first call, on a device. Neither shows
# up at build time, and no other check here looks at them — it is the empty
# module's failure mode one layer up.
dts=app/entry/src/main/cpp/types/librustcore/index.d.ts
napi=rust/bridge/csrc/napi_ohos.c
if [ -f "$dts" ] && [ -f "$napi" ]; then
	grep -o '^export function [a-zA-Z0-9_]*' "$dts" | awk '{print $3}' | sort >"$tmp/ts"
	grep -o '{"[a-zA-Z0-9_]*", NULL' "$napi" | sed -e 's/{"//' -e 's/", NULL//' | sort >"$tmp/c"
	if ! cmp -s "$tmp/ts" "$tmp/c"; then
		echo "index.d.ts and Init() disagree (< declared, > registered):" >&2
		cmp "$tmp/ts" "$tmp/c" >&2 || true
		exit 1
	fi
	echo "$(wc -l <"$tmp/ts" | tr -d ' ') functions, declared and registered"
else
	echo "no NAPI surface to compare (app not carried over)" >&2
fi

echo "== ArkTS shell, against the sibling =="
# The shell is this experiment's control variable: the same behaviour has to run
# on both cores, so a fix that lands on one side has to land on the other. It
# drifted once already (the strict todo-mutation path, the composer's return
# value, the todo error banner existed only here) and nothing noticed.
#
# Four files carry that behaviour; the rest of the shell is per-feature text,
# colour and layout, which is allowed to differ. Both copies are normalised to
# the same neutral names first (rustcore and gocore both become "gocore", both
# languages become "LANG"), then required to match byte for byte.
normalise() {
	sed -e 's/librustcore\.so/libgocore.so/g' \
		-e 's#types/librustcore#types/libgocore#g' \
		-e 's/RustCore/GoCore/g' \
		-e 's/rustcore/gocore/g' \
		-e 's/RustKit/GoKit/g' \
		-e 's/Rust/LANG/g' \
		-e 's/Go/LANG/g' "$1"
}
sib=../go-ohos-kit/app/entry/src/main/ets
if [ -d "$sib" ]; then
	for pair in \
		core/RustCore.ts:core/GoCore.ts \
		core/events.ets:core/events.ets \
		features/todos/TodoComposer.ets:features/todos/TodoComposer.ets \
		pages/Index.ets:pages/Index.ets; do
		here=${pair%%:*}
		there=${pair##*:}
		normalise "app/entry/src/main/ets/$here" >"$tmp/mine.ets"
		normalise "$sib/$there" >"$tmp/theirs.ets"
		if ! cmp -s "$tmp/mine.ets" "$tmp/theirs.ets"; then
			echo "$here drifted from $there — change both, or neither:" >&2
			diff "$tmp/mine.ets" "$tmp/theirs.ets" | head -20 >&2
			exit 1
		fi
	done
	echo "4 behaviour files match (names normalised)"
else
	echo "no sibling checkout: skipped"
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

	# `csrc/log_ohos.c` is the only thing here that calls `dup2` (it pipes fds 1
	# and 2 into hilog). Losing that object is silent in the worst way: the core's
	# eprintln! and any panic message go to descriptors the phone gives to nobody,
	# so the app fails with an empty log. Same failure class as the glue above.
	case $undefined in
	*dup2*) ;;
	*)
		echo "the .so does not reference dup2: the log pipe was dropped, hilog will be empty" >&2
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
