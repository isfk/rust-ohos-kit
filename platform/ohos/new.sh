#!/bin/sh
# Create an independent Rust + ArkTS project from the committed kit template.
# Usage: platform/ohos/new.sh <name> [dir] [--bundle com.example.name] [--label 'App Name']
# Ignored files (signing credentials, local SDK paths, build outputs) never enter it.
set -eu

usage() {
    echo 'usage: platform/ohos/new.sh <name> [dir] [--bundle com.example.name] [--label "App Name"]' >&2
}

name=
dir=
bundle=
label=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --bundle|--label)
            option=$1
            if [ "$#" -lt 2 ] || [ -z "$2" ]; then usage; exit 2; fi
            case $option in
                --bundle) bundle=$2 ;;
                --label) label=$2 ;;
            esac
            shift 2
            ;;
        -h|--help) usage; exit 0 ;;
        -*) usage; exit 2 ;;
        *)
            if [ -z "$name" ]; then
                name=$1
            elif [ -z "$dir" ]; then
                dir=$1
            else
                usage; exit 2
            fi
            shift
            ;;
    esac
done
if [ -z "$name" ]; then usage; exit 2; fi

# The name becomes both a path component and the default final bundle segment.
case $name in
    *[!a-z0-9_]*|[!a-z]*|'')
        echo 'new.sh: name must start with a lowercase letter and contain only a-z, 0-9 or _' >&2
        exit 2 ;;
esac
if [ -z "$bundle" ]; then bundle=com.example.$name; fi
if [ -z "$label" ]; then label=$name; fi
python3 - "$bundle" "$label" <<'PY'
import re
import sys

bundle, label = sys.argv[1:]
if not re.fullmatch(r'[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*){2,}', bundle):
    sys.exit('new.sh: bundle must be lowercase dotted segments, e.g. com.example.myapp')
if any(ord(c) < 32 for c in label):
    sys.exit('new.sh: label cannot contain control characters')
PY

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
if [ -z "$dir" ]; then dir=$root/apps/$name; fi
if [ -e "$dir" ] || [ -L "$dir" ]; then
    echo "new.sh: refusing to overwrite $dir" >&2
    exit 1
fi
if ! git -C "$root" rev-parse --verify 'HEAD^{commit}' >/dev/null 2>&1; then
    echo 'new.sh: a committed rust-ohos-kit template (HEAD) is required' >&2
    exit 1
fi
# Also fail closed if a future template accidentally tracks private files.
git -C "$root" ls-tree -r --name-only -z HEAD -- app rust .cargo | python3 -c '
import sys
paths = [p.decode() for p in sys.stdin.buffer.read().split(b"\0") if p]
for path in paths:
    name = path.rsplit("/", 1)[-1].lower()
    if path == "app/build-profile.json5" or name in {"local.properties", "secrets.json", "credentials.json", ".env"} or name.endswith((".p12", ".p7b", ".jks", ".keystore", ".pem", ".key")):
        sys.exit("new.sh: refusing a template containing tracked private files")
'

# Archive only committed, explicitly scoped files. Never copy .git, signing
# profiles, local.properties, DevEco caches, target/, or untracked user data.
mkdir -p "$dir"
git -C "$root" archive HEAD -- .gitignore Cargo.toml Cargo.lock .cargo rust app \
    platform/ohos/build.sh platform/ohos/run.sh | tar -xf - -C "$dir"

# A fresh bundle needs its own signature; this file is intentionally unsigned.
cp "$dir/app/build-profile.example.json5" "$dir/app/build-profile.json5"
python3 - "$dir" "$bundle" "$label" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
bundle, label = sys.argv[2:]
app = root / 'app/AppScope/app.json5'
source = app.read_text(encoding='utf-8')
old = '"bundleName": "cn.isfk.rustkit"'
if source.count(old) != 1:
    sys.exit('new.sh: unexpected template bundleName')
app.write_text(source.replace(old, f'"bundleName": {json.dumps(bundle)}'), encoding='utf-8')
for path, key in (
    ('app/AppScope/resources/base/element/string.json', 'app_name'),
    ('app/entry/src/main/resources/base/element/string.json', 'EntryAbility_label'),
):
    target = root / path
    data = json.loads(target.read_text(encoding='utf-8'))
    values = [item for item in data['string'] if item['name'] == key]
    if len(values) != 1:
        sys.exit(f'new.sh: unexpected template string {key}')
    values[0]['value'] = label
    target.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
PY

cat >"$dir/build.sh" <<'SH'
#!/bin/sh
# Build this project's Rust core and copy its .so into its ArkTS app.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec "$here/platform/ohos/build.sh" "$here/app" "$@"
SH
cat >"$dir/run.sh" <<'SH'
#!/bin/sh
# Build, sign (after DevEco setup), install and launch this app.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec "$here/platform/ohos/run.sh" "$here/app" "$@"
SH
chmod +x "$dir/build.sh" "$dir/run.sh" "$dir/platform/ohos/build.sh" "$dir/platform/ohos/run.sh"
cat >"$dir/README.md" <<'MD'
# Rust + ArkTS project

This project is copied from the committed `rust-ohos-kit` template. It includes
its own Rust workspace, ArkTS demo app, and build/run scripts. The source
checkout's signing certificates, local SDK settings and build artifacts are not
carried over. Internal crate and NAPI library names remain `rustkit-*` and
`librustcore.so`; change them together if you decide to rename the bridge.

1. Open `app/` in DevEco Studio and generate an automatic signing configuration
   for the bundle ID in `app/AppScope/app.json5`.
2. Run `./build.sh` to cross-compile the Rust library (requires DevEco SDK).
3. Run `./run.sh` to build a signed HAP and install it on a connected device.

The copied ArkTS UI is the kit demo (including its RustKit screen title);
customize its features and title for your app. No `ohpm install` is run by the generator.
MD
printf 'rustkit: created %s (bundle %s, unsigned)\n' "$dir" "$bundle"
printf '  Sign app/ in DevEco Studio, then run: cd %s && ./run.sh\n' "$dir" >&2
