#!/bin/sh
# Builds the Rust core, the app around it, installs it on the connected
# HarmonyOS device and starts it, then shows the core's log.
#
#   platform/ohos/run.sh [app project dir] [-bundle id] [-ability EntryAbility]
set -eu

project=${1:-app}
bundle=
ability=EntryAbility

while [ $# -gt 0 ]; do
	case $1 in
	-bundle) bundle=$2; shift 2 ;;
	-ability) ability=$2; shift 2 ;;
	-h | --help)
		sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) project=$1; shift ;;
	esac
done

sdk=${DEVECO_SDK_HOME:-/Users/shen/command-line-tools/sdk}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
project=$(cd "$project" 2>/dev/null && pwd) || {
	echo "rustkit: no app project at '$project'" >&2
	echo "rustkit: the ArkTS shell has not been carried over yet — see CONTEXT.md" >&2
	exit 1
}
hdc=$sdk/default/openharmony/toolchains/hdc

if [ -z "$bundle" ]; then
	bundle=$(sed -n 's/.*"bundleName"[^"]*"\([^"]*\)".*/\1/p' "$project/AppScope/app.json5")
fi

# build-profile.json5 is per-machine (DevEco writes this machine's cert paths and
# passwords into it), so it is not in the repo: start from the template.
if [ ! -f "$project/build-profile.json5" ]; then
	cp "$project/build-profile.example.json5" "$project/build-profile.json5"
	echo "rustkit: created build-profile.json5 from the template — it has no signing yet" >&2
	echo "rustkit: (DevEco → Project Structure → Signing Configs → Automatically generate signature)" >&2
fi

"$here/build.sh" "$project" >/dev/null

echo "rustkit: building the HAP" >&2
(cd "$project" && DEVECO_SDK_HOME="$sdk" "$sdk/../bin/hvigorw" assembleHap \
	--mode module -p product=default -p buildMode=debug --no-daemon >/dev/null)

hap=$project/entry/build/default/outputs/default/entry-default-signed.hap
if [ ! -f "$hap" ]; then
	echo "rustkit: the app isn't signed for $bundle." >&2
	echo "rustkit: open $project in DevEco once → Project Structure → Signing Configs →" >&2
	echo "rustkit: 'Automatically generate signature' (a Huawei ID), then run this again." >&2
	exit 1
fi

echo "rustkit: installing $bundle" >&2
"$hdc" install -r "$hap" >/dev/null

echo "rustkit: starting $ability" >&2
"$hdc" shell aa start -a "$ability" -b "$bundle" >/dev/null

echo "rustkit: log (hilog -T rustkit)" >&2
exec "$hdc" shell hilog -T rustkit
