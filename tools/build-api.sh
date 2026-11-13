#!/bin/sh
# build-api.sh - Build (or test) hbmgr, the Rust backend, in Docker
# Usage: sh tools/build-api.sh [build|test|check|fmt|lock] [version]
#
# Rust is not on the NAS: the official rust Alpine image builds a static musl binary into x86_64/bin/hbmgr.
# The cargo registry and target directory are kept under ./.cache so builds after the first are incremental.
# shared/ is mounted read-only beside the sources: the tests check the message catalogs in shared/web/i18n.
#   build  cargo build --release --locked, copied to x86_64/bin/hbmgr
#   test   cargo test
#   check  cargo fmt --check and cargo clippy -D warnings
#   fmt    cargo fmt
#   lock   cargo generate-lockfile

set -e
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
ROOT="$(dirname "$SCRIPT_DIR")"
. "$SCRIPT_DIR/docker.sh"

IMAGE="rust:1.99-alpine3.24"
MODE="${1:-build}"
VERSION="$2"
[ -n "$VERSION" ] || VERSION=$(sed -n 's/^QPKG_VER="\(.*\)"/\1/p' "$ROOT/qpkg.cfg" 2>/dev/null || true)
CACHE="$ROOT/.cache"
mkdir -p "$CACHE/registry" "$CACHE/target"

case "$MODE" in
	build) CMD="cargo build --release --locked && cp /work/target/release/hbmgr /out/hbmgr" ;;
	test) CMD="cargo test --release --locked" ;;
	check) CMD="cargo fmt --check && cargo clippy --release --locked --all-targets -- -D warnings" ;;
	fmt) CMD="cargo fmt" ;;
	lock) CMD="cargo generate-lockfile" ;;
	*) echo "usage: $0 [build|test|check|fmt|lock] [version]" >&2; exit 2 ;;
esac

mkdir -p "$ROOT/x86_64/bin"
$DOCKER run --rm \
	-e HBMGR_VERSION="$VERSION" -e CARGO_TARGET_DIR=/work/target \
	-v "$ROOT/api:/src" -v "$ROOT/shared:/shared:ro" -v "$CACHE/registry:/usr/local/cargo/registry" -v "$CACHE/target:/work/target" \
	-v "$ROOT/x86_64/bin:/out" -w /src "$IMAGE" \
	sh -c "set -e; rustup component add rustfmt clippy >/dev/null 2>&1 || true; $CMD"
