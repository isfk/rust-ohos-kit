#!/bin/sh
# The bridge numbers, without a finger on the Measure button.
#
#   platform/ohos/bench.sh [runs]      # default 5
#
# Starts the app with `aa start --pi bench N`, so the measurement runs inside the
# app exactly as it does when a person presses Measure — same ArkTS, same NAPI,
# same core — and reads the results out of hilog instead of off a screenshot.
#
# What this does not fix: the "bridge share" of the 300 KB path is one number
# subtracted from another, so its *multiple* is not quotable, only its direction.
# No amount of tooling changes arithmetic; that is why the summary marks it.
set -eu

runs=${1:-5}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

bundle=$(sed -n 's/.*"bundleName"[^"]*"\([^"]*\)".*/\1/p' app/AppScope/app.json5 2>/dev/null || true)
if [ -z "${bundle:-}" ]; then
	echo "no app/AppScope/app.json5 — carry the app over first" >&2
	exit 1
fi
hdc=${DEVECO_SDK_HOME:-/Users/shen/command-line-tools/sdk}/default/openharmony/toolchains/hdc
tag=rustkit-bench

echo "bench: $bundle, $runs rounds" >&2
"$hdc" shell aa force-stop "$bundle" >/dev/null 2>&1 || true
"$hdc" shell aa start -a EntryAbility -b "$bundle" --pi bench "$runs" >/dev/null

# Wait for the rounds to appear instead of sleeping a guessed number of seconds.
i=0
while [ "$i" -lt 30 ]; do
	sleep 1
	got=$("$hdc" shell hilog -x 2>/dev/null | grep -c "$tag bridge:" || true)
	[ "$got" -ge "$runs" ] && break
	i=$((i + 1))
done

lines=$("$hdc" shell hilog -x 2>/dev/null | grep -o "$tag bridge:.*" | tail -n "$runs")
if [ -z "$lines" ]; then
	echo "no '$tag bridge:' lines in hilog — is the app installed, and does it start?" >&2
	exit 1
fi

# The raw lines first: they are the evidence, and they are pasteable.
echo "$lines"
echo

# Then the summary. Fields are read by label, not by position, so the message can
# gain a clause without silently shifting a column.
printf '%s\n' "$lines" | awk '
	function median(a, n,    i, j, t) {
		# n is small (a handful of runs): insertion sort is plenty.
		for (i = 2; i <= n; i++) {
			t = a[i]
			for (j = i - 1; j >= 1 && a[j] > t; j--) a[j + 1] = a[j]
			a[j + 1] = t
		}
		return (n % 2) ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2
	}
	function row(name, a, n,    i, mn, mx) {
		mn = a[1]; mx = a[1]
		for (i = 2; i <= n; i++) { if (a[i] < mn) mn = a[i]; if (a[i] > mx) mx = a[i] }
		printf "  %-14s %8.1f %8.1f %8.1f\n", name, mn, median(a, n), mx
	}
	{
		for (i = 1; i < NF; i++) {
			if ($i == "int") int_ = $(i + 1)
			else if ($i == "json+parse") json = $(i + 1)
			else if ($i == "maths") maths = $(i + 1)
			else if ($i == "bridge") copy = $(i + 1)
		}
		n++; I[n] = int_; J[n] = json; M[n] = maths; C[n] = copy
	}
	END {
		if (n == 0) exit 1
		printf "  %-14s %8s %8s %8s   (µs)\n", "", "min", "median", "max"
		row("int back", I, n)
		row("JSON back", J, n)
		row("pixel maths", M, n)
		row("bridge copy", C, n)
		print ""
		print "  bridge copy is total minus the core'"'"'s own maths clock: the difference"
		print "  of two noisy numbers. Read its direction, not its multiple."
	}
'
