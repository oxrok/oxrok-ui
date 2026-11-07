#!/bin/sh
# 22 Packaging (Windows): stages the app with its DLLs, decoders and
# firmware, then builds the installer. Runs in an MSYS2 MINGW64 shell with
# mingw-w64-x86_64-{libsigrokdecode} (for the Python decoder files), a C
# compiler for the MinGW runtime and a Rust toolchain for
# x86_64-pc-windows-gnu.
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

cp -r /mingw64/share/libsigrokdecode/decoders "$SHARE/decoders"
mkdir -p "$SHARE/sigrok-firmware"
[ -d /mingw64/share/sigrok-firmware ] && cp -r /mingw64/share/sigrok-firmware/. "$SHARE/sigrok-firmware/"
[ -d vendor/sigrok-firmware ] && cp -r vendor/sigrok-firmware/. "$SHARE/sigrok-firmware/"
[ -d vendor/sigrok-util ] && cp -r vendor/sigrok-util "$SHARE/sigrok-util" && rm -rf "$SHARE/sigrok-util/.git"

"$STAGE/bin/oxrok-ui.exe" --self-test

# The installer (Inno Setup's iscc on PATH).
iscc //DVersion="$VERSION" //DStage="$(cygpath -w "$STAGE")" //O"$(cygpath -w "$OUT")" packaging/windows/installer.iss
