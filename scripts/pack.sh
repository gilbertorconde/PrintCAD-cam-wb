#!/usr/bin/env bash
# Build the component and pack it with its manifest and icons into
# dist/<id>-<version>.pcbench, the file printCAD installs.
#
#   scripts/pack.sh            build and pack
#   scripts/pack.sh v0.2.0     also check that the tag names bench.toml's version
set -euo pipefail
cd "$(dirname "$0")/.."

field() { sed -n "s/^$1 *= *\"\(.*\)\"/\1/p" "$2" | head -n1; }
id=$(field id bench.toml)
version=$(field version bench.toml)
crate=$(field name Cargo.toml)
crate_version=$(field version Cargo.toml)

if [ "$version" != "$crate_version" ]; then
  echo "bench.toml says $version and Cargo.toml $crate_version; make them agree" >&2
  exit 1
fi
if [ $# -ge 1 ] && [ "${1#v}" != "$version" ]; then
  echo "tag $1 does not name version $version from bench.toml" >&2
  exit 1
fi

cargo build --release
wasm="${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/${crate//-/_}.wasm"

stage=dist/package
rm -rf "$stage"
mkdir -p "$stage/icons"
cp bench.toml "$stage/"
cp "$wasm" "$stage/bench.wasm"
cp icons/*.svg "$stage/icons/"
cp README.md "$stage/"

out="dist/$id-$version.pcbench"
tar czf "$out" -C "$stage" bench.toml bench.wasm README.md icons
echo "$out"
sha256sum "$out"
