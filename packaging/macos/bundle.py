#!/usr/bin/env python3
"""Builds "Oxrok UI.app" from a release binary (22 Packaging, macOS).

oxrok and RustPython are in the binary, so there are no libraries to copy.
Resources: the decoders of a libsigrokdecode installation (oxrok runs them
on RustPython), vendor/ firmware (make firmware-deps) and the sigrok-util
extraction scripts. Signs ad-hoc and makes a .dmg.

    bundle.py --bin target/release/oxrok-ui --version 0.11.0 --out dist [--vendor vendor]
"""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

APP_NAME = "Oxrok UI"
BUNDLE_ID = "org.oxrok.ui"


def run(*cmd, check=True):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"{' '.join(cmd)}: {r.stderr.strip()}")
    return r.stdout


def decoders_dir():
    """The decoders of a libsigrokdecode installation, if there is one."""
    for p in (
        "/opt/homebrew/share/libsigrokdecode/decoders",
        "/usr/local/share/libsigrokdecode/decoders",
        "/usr/share/libsigrokdecode/decoders",
    ):
        if Path(p).is_dir():
            return Path(p)
    prefix = run("brew", "--prefix", "libsigrokdecode", check=False).strip()
    if prefix and (Path(prefix) / "share/libsigrokdecode/decoders").is_dir():
        return Path(prefix) / "share/libsigrokdecode/decoders"
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True, type=Path)
    ap.add_argument("--version", required=True)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--vendor", type=Path, default=Path("vendor"))
    ap.add_argument("--icon", type=Path, default=Path("assets/AppIcon.icns"))
    a = ap.parse_args()

    app = a.out / f"{APP_NAME}.app"
    if app.exists():
        shutil.rmtree(app)
    (app / "Contents/MacOS").mkdir(parents=True)
    res = app / "Contents/Resources"
    res.mkdir()
    binary = app / "Contents/MacOS/oxrok-ui"
    shutil.copy2(a.bin, binary)
    shutil.copy2(a.icon, res / "AppIcon.icns")
    # Written by hand: plistlib needs pyexpat, broken in some Pythons.
    keys = {
        "CFBundleName": APP_NAME, "CFBundleDisplayName": APP_NAME, "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleExecutable": "oxrok-ui", "CFBundleIconFile": "AppIcon", "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": a.version, "CFBundleVersion": a.version,
        "LSMinimumSystemVersion": "12.0",
    }
    body = "".join(f"\t<key>{k}</key>\n\t<string>{v}</string>\n" for k, v in keys.items())
    (app / "Contents/Info.plist").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
        f'<plist version="1.0">\n<dict>\n{body}\t<key>NSHighResolutionCapable</key>\n\t<true/>\n</dict>\n</plist>\n'
    )

    dec = decoders_dir()
    if dec:
        shutil.copytree(dec, res / "decoders", ignore=shutil.ignore_patterns("__pycache__"))
    else:
        print("  ! no libsigrokdecode decoders found; the bundle has none", file=sys.stderr)
    fw = res / "sigrok-firmware"
    fw.mkdir()
    for share in ("/opt/homebrew/share/sigrok-firmware", "/usr/local/share/sigrok-firmware"):
        if Path(share).is_dir():
            shutil.copytree(share, fw, dirs_exist_ok=True)
    if (a.vendor / "sigrok-firmware").is_dir():
        shutil.copytree(a.vendor / "sigrok-firmware", fw, dirs_exist_ok=True)
    if (a.vendor / "sigrok-util").is_dir():
        shutil.copytree(a.vendor / "sigrok-util", res / "sigrok-util", ignore=shutil.ignore_patterns(".git"))

    # Ad-hoc signature, then the app.
    run("codesign", "--force", "--sign", "-", str(binary))
    run("codesign", "--force", "--sign", "-", str(app))

    dmg = a.out / f"Oxrok-UI-{a.version}-macos-{os.uname().machine}.dmg"
    if dmg.exists():
        dmg.unlink()
    run("hdiutil", "create", "-volname", APP_NAME, "-srcfolder", str(app), "-ov", "-format", "UDZO", str(dmg))
    print(f"{app}\n{dmg}")


if __name__ == "__main__":
    main()
