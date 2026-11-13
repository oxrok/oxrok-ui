#!/bin/sh
# 22 Packaging (Linux): an AppImage from a release binary.
#
#   packaging/linux/appimage.sh target/release/oxrok-ui 0.11.0 dist
#
# Needs: appimagetool on PATH (CI downloads it), ldd. oxrok and RustPython
# are in the binary; the decoders of a libsigrokdecode installation,
# vendor/ (make firmware-deps) and the USB permission rules go in.
set -eu
BIN=$1
VERSION=$2
OUT=$3
ARCH=$(uname -m)
APPDIR=$OUT/AppDir
SHARE=$APPDIR/usr/share/oxrok-ui
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$SHARE"
cp "$BIN" "$APPDIR/usr/bin/oxrok-ui"

# Libraries: everything ldd resolves except the base system every distro
# has (the AppImage excludelist's core: glibc, the GPU and windowing stack).
EXCLUDE='^(linux-vdso|ld-linux|libc|libm|libdl|libpthread|librt|libresolv|libutil|libgcc_s|libstdc\+\+|libGL|libEGL|libGLX|libGLdispatch|libvulkan|libwayland|libxkbcommon|libX|libxcb|libdrm|libgbm|libudev|libsystemd|libasound|libfontconfig|libfreetype|libz)\b'
ldd "$BIN" | awk '/=> \//{print $1, $3}' | while read -r name path; do
    if ! echo "$name" | grep -Eq "$EXCLUDE"; then
        cp -L "$path" "$APPDIR/usr/lib/"
    fi
done

# The Python decoders, which oxrok runs on RustPython.
for d in /usr/share/libsigrokdecode/decoders /usr/local/share/libsigrokdecode/decoders; do
    [ -d "$d" ] && cp -r "$d" "$SHARE/decoders" && break
done
mkdir -p "$SHARE/sigrok-firmware"
for d in /usr/share/sigrok-firmware /usr/local/share/sigrok-firmware; do
    [ -d "$d" ] && cp -r "$d/." "$SHARE/sigrok-firmware/"
done
[ -d vendor/sigrok-firmware ] && cp -r vendor/sigrok-firmware/. "$SHARE/sigrok-firmware/"

# USB permission rules, for the user to install.
mkdir -p "$SHARE/udev"
for r in /usr/lib/udev/rules.d/*sigrok*.rules /lib/udev/rules.d/*sigrok*.rules; do
    [ -f "$r" ] && cp "$r" "$SHARE/udev/"
done
cat > "$SHARE/udev/README" <<'TXT'
USB logic analyzers need permission to be opened. Install these rules once:

    sudo cp *.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules

then replug the device.
TXT

cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$HERE/usr/bin/oxrok-ui" "$@"
RUN
chmod +x "$APPDIR/AppRun"
cat > "$APPDIR/oxrok-ui.desktop" <<'DESK'
[Desktop Entry]
Type=Application
Name=Oxrok UI
Exec=oxrok-ui
Icon=oxrok-ui
Categories=Development;Electronics;
DESK
cp assets/icon.png "$APPDIR/oxrok-ui.png"

IMAGE="$OUT/Oxrok-UI-$VERSION-linux-$ARCH.AppImage"
ARCH=$ARCH appimagetool "$APPDIR" "$IMAGE"
echo "$IMAGE"
