#!/bin/sh
# build.sh - Build the Homebridge QPKG on a QNAP NAS
# Usage: sh build.sh
#
# Needs Docker (Container Station: hbmgr is built in the official rust Alpine image by tools/build-api.sh) and
# qbuild. The JavaScript syntax check runs when node is on the PATH. v1 ships x86_64 only.
# SKIP_API=1 reuses an existing x86_64/bin/hbmgr.

set -e

export PATH="/usr/local/apache/bin:/usr/local/bin:/usr/bin:/bin:$PATH"

SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
cd "$SCRIPT_DIR"

VERSION=$(sed -n 's/^QPKG_VER="\(.*\)"/\1/p' qpkg.cfg)

echo "=== Homebridge QPKG Build (version $VERSION) ==="
echo ""

echo "Cleaning macOS junk files..."
find . -path ./.cache -prune -o \( -name '.DS_Store' -o -name '._*' \) -print -exec rm -f {} + 2>/dev/null || true
find . -path ./.cache -prune -o -name '__MACOSX' -type d -print -exec rm -rf {} + 2>/dev/null || true
echo ""

echo "Checking shell syntax..."
for f in shared/Homebridge.sh package_routines build.sh tools/*.sh; do
	if sh -n "$f"; then
		echo "  OK: $f"
	else
		echo "  FAIL: $f"
		exit 1
	fi
done
echo ""

echo "Checking the web UI..."
if [ ! -f shared/web/index.html ]; then
	echo "  FAIL: shared/web/index.html is missing."
	exit 1
fi
if command -v node >/dev/null 2>&1; then
	for f in shared/web/js/*.js; do
		[ -f "$f" ] || continue
		if node --check "$f"; then
			echo "  OK: $f"
		else
			echo "  FAIL: $f"
			exit 1
		fi
	done
else
	echo "  skipped the JavaScript syntax check (node is not on the PATH)"
fi
echo ""

echo "Building hbmgr..."
if [ "$SKIP_API" = "1" ] && [ -x x86_64/bin/hbmgr ]; then
	echo "  skipped (SKIP_API=1): $(x86_64/bin/hbmgr --version)"
else
	mkdir -p build
	if ! sh tools/build-api.sh test "$VERSION" > build/rust-test.log 2>&1; then
		grep -E '^test |FAILED|panicked|^error' build/rust-test.log | grep -v ' ok$' | tail -40
		echo "  FAIL: Rust tests (full output in build/rust-test.log)"
		exit 1
	fi
	grep -E '^test result' build/rust-test.log
	sh tools/build-api.sh build "$VERSION"
	echo "  OK: $(x86_64/bin/hbmgr --version)"
fi
BUILT_VERSION=$(x86_64/bin/hbmgr --version | sed 's/^hbmgr //')
if [ "$BUILT_VERSION" != "$VERSION" ]; then
	echo "  FAIL: x86_64/bin/hbmgr is version $BUILT_VERSION, qpkg.cfg says $VERSION (build without SKIP_API=1)."
	exit 1
fi
echo ""

# The package is installed on other NAS units: nothing from this machine.
echo "Checking the package is clean..."
CLEAN_ERR=0
if [ -d shared/data ]; then
	echo "  FAIL: shared/data exists (runtime data from a local test)."
	CLEAN_ERR=1
fi
if [ -n "$(ls -A x86_64/bin 2>/dev/null | grep -v '^hbmgr$')" ]; then
	echo "  FAIL: unexpected files in x86_64/bin: $(ls -A x86_64/bin | grep -v '^hbmgr$' | tr '\n' ' ')"
	CLEAN_ERR=1
fi
LEFTOVERS=$(find shared icons x86_64 \( -name '*.log' -o -name '*.pid' -o -name '*.lock' -o -name '*.secret' -o -name '*.key' \
	-o -name '*.pem' -o -name '*.env' -o -name '*.bak*' -o -name '*.tmp' -o -name '*.tmp-*' -o -name '*.orig' \) -print)
if [ -n "$LEFTOVERS" ]; then
	echo "  FAIL: leftover runtime or secret files:"
	echo "$LEFTOVERS" | sed 's/^/    /'
	CLEAN_ERR=1
fi
LOCAL_PATHS=$(find shared -type f ! -name '*.png' ! -name '*.gif' ! -name '*.ico' -print | xargs grep -l -e '/share/CACHEDEV' -e '/mnt/HDA_ROOT' -e '\.devtest' 2>/dev/null || true)
if [ -n "$LOCAL_PATHS" ]; then
	echo "  FAIL: paths of this NAS in:"
	echo "$LOCAL_PATHS" | sed 's/^/    /'
	CLEAN_ERR=1
fi
SECRETS=$(find shared -type f ! -name '*.png' ! -name '*.gif' ! -name '*.ico' -print | xargs grep -l -E 'glpat-|NAS_SID=[A-Za-z0-9]{6,}|BEGIN [A-Z ]*PRIVATE KEY' 2>/dev/null || true)
if [ -n "$SECRETS" ]; then
	echo "  FAIL: what looks like a secret in:"
	echo "$SECRETS" | sed 's/^/    /'
	CLEAN_ERR=1
fi
if [ "$CLEAN_ERR" -eq 1 ]; then
	echo "Package is not clean. Aborting build."
	exit 1
fi
echo "  OK"
echo ""

echo "Cleaning previous build output..."
rm -f build/*.qpkg build/*.md5 2>/dev/null
echo ""

echo "Running qbuild..."
qbuild --build-arch x86_64
echo ""

echo "=== Build Results ==="
if ls build/*.qpkg 1>/dev/null 2>&1; then
	ls -lh build/*.qpkg
	echo ""
	echo "Build successful!"
else
	echo "No .qpkg file found in build/. Build may have failed."
	exit 1
fi
