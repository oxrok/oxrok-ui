#!/bin/sh
# 22 Packaging (Windows): stages the app with its DLLs, decoders and
# firmware, then builds the installer. Runs in an MSYS2 MINGW64 shell with
# a C compiler for the MinGW runtime and a Rust toolchain for
# x86_64-pc-windows-gnu. DECODERS is the decoders directory of a
# libsigrokdecode release (the Python decoder files), which MSYS2 does not
# package; without it, the one of an installation under /mingw64.
#
#   packaging/windows/build.sh 0.11.0 dist
#
# Layout (the app finds its resources at ../share/oxrok-ui):
#   bin/oxrok-ui.exe, bin/*.dll, share/oxrok-ui/{decoders,sigrok-firmware,…}
set -eu
VERSION=$1
OUT=$2
TARGET=x86_64-pc-windows-gnu
cargo build --release --target $TARGET
STAGE=$OUT/stage
SHARE=$STAGE/share/oxrok-ui
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$SHARE"
EXE=target/$TARGET/release/oxrok-ui.exe
cp "$EXE" "$STAGE/bin/"

# Every MinGW DLL the exe needs (ldd lists them with their /mingw64 paths).
ldd "$EXE" | awk '/\/mingw64\//{print $3}' | sort -u | while read -r dll; do
    cp "$dll" "$STAGE/bin/"
done

DECODERS=${DECODERS:-/mingw64/share/libsigrokdecode/decoders}
[ -d "$DECODERS" ] || { echo "no decoders in $DECODERS: set DECODERS" >&2; exit 1; }
cp -r "$DECODERS" "$SHARE/decoders"
mkdir -p "$SHARE/sigrok-firmware"
[ -d /mingw64/share/sigrok-firmware ] && cp -r /mingw64/share/sigrok-firmware/. "$SHARE/sigrok-firmware/"
[ -d vendor/sigrok-firmware ] && cp -r vendor/sigrok-firmware/. "$SHARE/sigrok-firmware/"

"$STAGE/bin/oxrok-ui.exe" --self-test

# The installer (Inno Setup's iscc on PATH).
# Absolute paths: iscc reads a relative one from the script's directory.
iscc //DVersion="$VERSION" //DStage="$(cygpath -aw "$STAGE")" //O"$(cygpath -aw "$OUT")" packaging/windows/installer.iss
