#!/usr/bin/env bash
# Build the component and pack it with its manifest, icons and native helpers
# into dist/<id>-<version>.pcbench, the file printCAD installs.
#
#   scripts/pack.sh            build and pack, with the helper built for this machine
#   scripts/pack.sh v0.2.0     also check that the tag names bench.toml's version
#
# HELPERS=<dir> packs the helpers already built in <dir>, laid out as the
# package has them (<dir>/<os>-<arch>/camcraft-helper[.exe]), instead of
# building one: the release workflow builds one on each system and packs them
# all.
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

target="${CARGO_TARGET_DIR:-target}"
cargo build --release -p "$crate" --target wasm32-wasip2
wasm="$target/wasm32-wasip2/release/${crate//-/_}.wasm"

stage=dist/package
rm -rf "$stage"
mkdir -p "$stage/icons" "$stage/helpers"
cp bench.toml "$stage/"
cp "$wasm" "$stage/bench.wasm"
cp icons/*.svg "$stage/icons/"
cp README.md "$stage/"
# camcraft's licences and notice travel with the engine built from it.
mkdir -p "$stage/helpers/camcraft"
cp camcraft/LICENSE-MIT camcraft/LICENSE-APACHE camcraft/NOTICE "$stage/helpers/camcraft/"

if [ -n "${HELPERS:-}" ]; then
  cp -R "$HELPERS"/. "$stage/helpers/"
else
  # The folder printCAD looks in: Rust's names for the system and the machine.
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*) os=windows ;;
    *) echo "no helper folder for $(uname -s)" >&2; exit 1 ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) echo "no helper folder for $(uname -m)" >&2; exit 1 ;;
  esac
  host=$(rustc -vV | sed -n 's/^host: //p')
  cargo build --release -p camcraft-helper --target "$host"
  exe=""
  [ "$os" = windows ] && exe=.exe
  mkdir -p "$stage/helpers/$os-$arch"
  cp "$target/$host/release/camcraft-helper$exe" "$stage/helpers/$os-$arch/"
fi
chmod +x "$stage"/helpers/*/camcraft-helper* 2>/dev/null || true
ls "$stage"/helpers/*/camcraft-helper* >/dev/null

out="dist/$id-$version.pcbench"
tar czf "$out" -C "$stage" bench.toml bench.wasm README.md icons helpers
echo "$out"
if command -v sha256sum >/dev/null; then sha256sum "$out"; else shasum -a 256 "$out"; fi
