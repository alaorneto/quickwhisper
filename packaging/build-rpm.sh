#!/usr/bin/env bash
# Build the QuickWhisper RPM from the current working tree (uncommitted
# changes included). Requires: rpm-build, cmake, gcc-c++, alsa-lib-devel and
# a Rust toolchain (rustup is fine).
#
#   ./packaging/build-rpm.sh
#   sudo dnf install target/rpmbuild/RPMS/x86_64/quickwhisper-*.rpm
set -euo pipefail
cd "$(dirname "$0")/.."

command -v rpmbuild >/dev/null 2>&1 || {
    echo 'error: rpmbuild not found — run: sudo dnf install rpm-build' >&2
    exit 1
}

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
topdir=$PWD/target/rpmbuild
mkdir -p "$topdir/SOURCES"

tar czf "$topdir/SOURCES/quickwhisper-$version.tar.gz" \
    --transform "s,^,quickwhisper-$version/," \
    Cargo.toml Cargo.lock LICENSE README.md src data extension packaging

rpmbuild --define "_topdir $topdir" -bb packaging/quickwhisper.spec

echo
echo "RPM pronto:"
find "$topdir/RPMS" -name '*.rpm'
