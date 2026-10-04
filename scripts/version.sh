#!/bin/sh
# Prints the current version, or sets a new one: scripts/version.sh 0.2.0
# The version lives in [workspace.package] in Cargo.toml; everything else reads it from there.
set -e
cd "$(dirname "$0")/.."
current=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
if [ -z "$1" ]; then
    echo "$current"
    exit 0
fi
case "$1" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "usage: $0 [MAJOR.MINOR.PATCH]" >&2; exit 1 ;;
esac
sed -i '' "s/^version = \"$current\"/version = \"$1\"/" Cargo.toml
cargo update --workspace --quiet
echo "$current -> $1. Commit, then tag with: git tag v$1"
