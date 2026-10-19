#!/usr/bin/env bash
# Usage: scripts/release.sh 0.2.0
# Bumps the version everywhere, commits, tags v0.2.0 and pushes. GitHub
# Actions then builds, signs, notarizes and publishes the release.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/release.sh X.Y.Z" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "Commit or stash your changes first." >&2
  exit 1
fi

python3 - "$version" <<'PY'
import json, re, sys
v = sys.argv[1]
for path in ("package.json", "src-tauri/tauri.conf.json"):
    data = json.load(open(path))
    data["version"] = v
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
toml = open("src-tauri/Cargo.toml").read()
toml = re.sub(r'(?m)^version = "[^"]+"', f'version = "{v}"', toml, count=1)
open("src-tauri/Cargo.toml", "w").write(toml)
PY

(cd src-tauri && cargo update -p butters --offline >/dev/null 2>&1 || cargo update -p butters >/dev/null)
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Release v$version"
git tag "v$version"
git push origin HEAD "v$version"
echo "Pushed v$version. Watch it: gh run watch"
