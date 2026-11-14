#!/usr/bin/env python3
"""Builds Oxrok.app from a release binary (22 Packaging, macOS).

oxrok and RustPython are in the binary, so there are no libraries to copy.
Resources: the decoders of a libsigrokdecode installation (oxrok runs them
on RustPython) and vendor/ firmware (make firmware-deps). Signs and makes
a .dmg, whose window has the app beside a link to /Applications on
dmg/background.svg.

    bundle.py --bin target/release/oxrok-ui --version 0.11.0 --out dist [--vendor vendor] [--cli target/oxrok-cli/bin/oxrok]
              [--sign <identity> [--notary-profile <name> | --notary-key AuthKey.p8 --notary-key-id <id> --notary-issuer <uuid>]]

With --cli the oxrok CLI is bundled in Contents/MacOS and signed as a leaf
before the app.

Signing: without --sign the app is signed ad-hoc. With --sign (a Developer
ID Application identity: its name, or its hash where two have the name) the
code gets the hardened runtime and a secure timestamp, which Apple's notary
service asks for, and the .dmg is signed too. It needs no entitlements:
nothing is loaded from outside the bundle and RustPython interprets.

Notarization: with --notary-profile (a keychain profile of `xcrun notarytool
store-credentials`) or the three --notary-key options (an App Store Connect
API key), the app and then the .dmg go to the notary service and get the
ticket stapled, so both open on a Mac that is offline.

The .dmg's window: Finder lays it out (osascript), which needs a logged-in
desktop and, the first time, the permission to control Finder.

The work is in two stages, for a build that keeps the signing keys away
from the code it builds (.github/workflows/package.yml):

    bundle.py --stage app --bin … --version … --out dist    # the app, signed ad-hoc
    bundle.py --stage package --out dist --sign …           # signs it again, notarizes, makes the .dmg
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# The app's name, as it calls itself in its menus and windows.
APP_NAME = "Oxrok UI"
# The bundle in dist/, in the .dmg and in /Applications.
BUNDLE = "Oxrok.app"
BUNDLE_ID = "org.oxrok.ui"

# The .dmg's window, in points: its content, and the middles of the two
# icons on dmg/background.svg.
DMG_ART = Path(__file__).resolve().parent / "dmg"
DMG_WINDOW = (660, 400)
DMG_TITLE_BAR = 28
DMG_ICON = 128
DMG_APP_AT = (170, 165)
DMG_APPLICATIONS_AT = (490, 165)

# Finder writes what it is told here to the volume's .DS_Store.
DMG_LAYOUT = """\
on run argv
	set volumeName to item 1 of argv
	tell application "Finder"
		tell disk volumeName
			open
			tell container window
				set current view to icon view
				set toolbar visible to false
				set statusbar visible to false
				set the bounds to {%(left)d, %(top)d, %(right)d, %(bottom)d}
			end tell
			set viewOptions to the icon view options of container window
			tell viewOptions
				set arrangement to not arranged
				set icon size to %(icon)d
				set text size to 13
			end tell
			set background picture of viewOptions to file ".background:background.tiff"
			set position of item "%(bundle)s" to {%(app_x)d, %(app_y)d}
			set position of item "Applications" to {%(applications_x)d, %(applications_y)d}
			close
			open
			update without registering applications
			delay 2
			tell container window
				set statusbar visible to false
				set the bounds to {%(left)d, %(top)d, %(right)d, %(bottom)d}
			end tell
			delay 2
			close
		end tell
	end tell
end run
"""


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


def assemble(a, app):
    """The .app from the release binaries and the resources."""
    if app.exists():
        shutil.rmtree(app)
    (app / "Contents/MacOS").mkdir(parents=True)
    res = app / "Contents/Resources"
    res.mkdir()
    binary = app / "Contents/MacOS/oxrok-ui"
    shutil.copy2(a.bin, binary)
    cli = None
    if a.cli:
        cli = app / "Contents/MacOS/oxrok"
        shutil.copy2(a.cli, cli)
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


def sign(app, identity):
    """Signs each Mach-O leaf first, then the app (ad-hoc without an identity)."""
    # The hardened runtime and the timestamp are for a real identity: the
    # notary service asks for both, and an ad-hoc signature has neither.
    how = ["--sign", identity, "--options", "runtime", "--timestamp"] if identity else ["--sign", "-"]
    macos = app / "Contents/MacOS"
    main_binary = macos / "oxrok-ui"
    for b in sorted(p for p in macos.iterdir() if p != main_binary) + [main_binary]:
        run("codesign", "--force", *how, str(b))
    run("codesign", "--force", *how, str(app))
    run("codesign", "--verify", "--strict", "--deep", str(app))


def notarize(path, creds):
    """Sends a .zip or .dmg to Apple's notary service and waits for its answer."""
    cmd = ["xcrun", "notarytool", "submit", str(path), "--wait", "--timeout", "45m", "--output-format", "json", *creds]
    r = subprocess.run(cmd, capture_output=True, text=True)
    try:
        answer = json.loads(r.stdout)
    except ValueError:
        answer = {}
    if answer.get("status") != "Accepted":
        # The service's log says which file it refused, and why.
        log = run("xcrun", "notarytool", "log", answer["id"], *creds, check=False) if answer.get("id") else ""
        sys.exit(f"notarization of {path.name}: {answer.get('status') or r.stderr.strip() or r.stdout.strip()}\n{log}")
    print(f"  notarized {path.name} ({answer['id']})", file=sys.stderr)


def app_version(app):
    """The version the app was assembled with."""
    plist = (app / "Contents/Info.plist").read_text()
    m = re.search(r"<key>CFBundleShortVersionString</key>\s*<string>([^<]+)</string>", plist)
    if not m:
        sys.exit(f"{app}: no version in its Info.plist")
    return m.group(1)


def finder_layout(volume, mount):
    """Has Finder lay out the mounted volume's window."""
    left, top = 200, 120
    script = DMG_LAYOUT % {
        "left": left, "top": top, "right": left + DMG_WINDOW[0], "bottom": top + DMG_TITLE_BAR + DMG_WINDOW[1],
        "icon": DMG_ICON, "bundle": BUNDLE,
        "app_x": DMG_APP_AT[0], "app_y": DMG_APP_AT[1],
        "applications_x": DMG_APPLICATIONS_AT[0], "applications_y": DMG_APPLICATIONS_AT[1],
    }
    # Finder sometimes lets an Apple event time out, on a CI runner above all.
    said = ""
    for _ in range(3):
        try:
            r = subprocess.run(["osascript", "-", volume], input=script, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            said = "no answer in 5 minutes"
            continue
        if r.returncode == 0:
            break
        said = r.stderr.strip()
        print(f"  ! Finder: {said}; trying again", file=sys.stderr)
        time.sleep(3)
    else:
        sys.exit(f"Finder did not lay out the .dmg's window: {said}\n"
                 "(it needs a logged-in desktop, and the permission to control Finder: "
                 "System Settings, Privacy & Security, Automation)")
    # Finder writes the layout a little after it is told.
    store = mount / ".DS_Store"
    for _ in range(30):
        if store.is_file() and store.stat().st_size:
            break
        time.sleep(1)
    else:
        sys.exit(f"Finder did not write the .dmg's window layout ({store})")
    run("sync")
    time.sleep(2)


def disk_image(app, dmg):
    """The .dmg: the app beside a link to /Applications, in a window with dmg/background.svg behind."""
    art = [DMG_ART / "background.png", DMG_ART / "background@2x.png"]
    for f in art:
        if not f.is_file():
            sys.exit(f"{f}: the .dmg's background is not there")
    work = Path(tempfile.mkdtemp(prefix="dmg-", dir=dmg.parent))
    try:
        stage = work / "stage"
        (stage / ".background").mkdir(parents=True)
        # One TIFF with both resolutions: Finder picks by the display.
        run("tiffutil", "-cathidpicheck", *map(str, art), "-out", str(stage / ".background/background.tiff"))
        run("ditto", str(app), str(stage / BUNDLE))
        os.symlink("/Applications", stage / "Applications")

        # Read-write first, with room for what Finder writes.
        megabytes = int(run("du", "-sk", str(stage)).split()[0]) // 1024
        image = work / "layout.dmg"
        run("hdiutil", "create", "-volname", APP_NAME, "-srcfolder", str(stage), "-fs", "HFS+", "-format", "UDRW",
            "-size", f"{megabytes + megabytes // 8 + 32}m", str(image))
        device = mount = None
        for line in run("hdiutil", "attach", "-readwrite", "-noverify", "-noautoopen", str(image)).splitlines():
            fields = [f.strip() for f in line.split("\t")]
            device = device or fields[0]
            if fields[-1].startswith("/Volumes/"):
                mount = Path(fields[-1])
        if not (device and mount):
            sys.exit(f"{image}: did not mount")
        try:
            # Finder knows a volume by its name, and a second one of the
            # name is mounted as "<name> 1".
            if mount.name != APP_NAME:
                sys.exit(f"a volume named {APP_NAME} is mounted already: eject it first")
            finder_layout(APP_NAME, mount)
            shutil.rmtree(mount / ".fseventsd", ignore_errors=True)
        finally:
            for attempt in range(5):
                if subprocess.run(["hdiutil", "detach", device, *(["-force"] if attempt == 4 else [])],
                                  capture_output=True).returncode == 0:
                    break
                time.sleep(2)
            else:
                sys.exit(f"{device}: could not eject the .dmg being laid out")
        run("hdiutil", "convert", str(image), "-format", "UDZO", "-imagekey", "zlib-level=9", "-o", str(dmg))
    finally:
        shutil.rmtree(work, ignore_errors=True)


def package(a, app):
    """Signs the app, has it notarized, and makes the .dmg of it."""
    creds = []
    if a.notary_profile:
        creds = ["--keychain-profile", a.notary_profile]
    elif a.notary_key:
        creds = ["--key", str(a.notary_key), "--key-id", a.notary_key_id, "--issuer", a.notary_issuer]
    sign(app, a.sign)
    if creds:
        # The app first, so that the one in the .dmg has its ticket.
        zipped = (a.out / BUNDLE).with_suffix(".zip")
        run("ditto", "-c", "-k", "--keepParent", str(app), str(zipped))
        try:
            notarize(zipped, creds)
        finally:
            zipped.unlink()
        run("xcrun", "stapler", "staple", str(app))

    dmg = a.out / f"Oxrok-UI-{a.version or app_version(app)}-macos-{os.uname().machine}.dmg"
    if dmg.exists():
        dmg.unlink()
    disk_image(app, dmg)
    if a.sign:
        run("codesign", "--force", "--sign", a.sign, "--timestamp", str(dmg))
    if creds:
        notarize(dmg, creds)
        run("xcrun", "stapler", "staple", str(dmg))
        # What Gatekeeper will say of a download.
        run("spctl", "--assess", "--type", "execute", str(app))
        run("spctl", "--assess", "--type", "open", "--context", "context:primary-signature", str(dmg))
    return dmg


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--stage", choices=("all", "app", "package"), default="all",
                    help="app: only the .app, signed ad-hoc; package: sign the .app in --out, notarize it and make the .dmg; all: both")
    ap.add_argument("--bin", type=Path, help="the release binary (not for --stage package)")
    ap.add_argument("--version", help="the app's version (--stage package reads it from the app)")
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--vendor", type=Path, default=Path("vendor"))
    ap.add_argument("--icon", type=Path, default=Path("assets/AppIcon.icns"))
    ap.add_argument("--sign", default="", help="codesign identity (e.g. from `security find-identity -v -p codesigning`); ad-hoc when empty")
    ap.add_argument("--cli", type=Path, default=None, help="the oxrok CLI binary to bundle in Contents/MacOS and sign")
    ap.add_argument("--notary-profile", default="", help="notarize with this keychain profile of `xcrun notarytool store-credentials`")
    ap.add_argument("--notary-key", type=Path, default=None, help="notarize with this App Store Connect API key (.p8)")
    ap.add_argument("--notary-key-id", default="")
    ap.add_argument("--notary-issuer", default="")
    a = ap.parse_args()
    if a.stage != "package" and not (a.bin and a.version):
        ap.error("--bin and --version are required to assemble the app")
    notary = bool(a.notary_profile or a.notary_key)
    if a.notary_key and not (a.notary_key_id and a.notary_issuer):
        ap.error("--notary-key needs --notary-key-id and --notary-issuer")
    if notary and not a.sign:
        ap.error("the notary service takes only what a Developer ID signed: give --sign")
    if a.stage == "app" and (a.sign or notary):
        ap.error("--stage app signs ad-hoc; --sign and the notary options are for the package")

    app = a.out / BUNDLE
    if a.stage != "package":
        assemble(a, app)
    elif not app.is_dir():
        sys.exit(f"{app}: not there; --stage app makes it")
    if a.stage == "app":
        sign(app, "")
        print(app)
        return
    dmg = package(a, app)
    print(f"{app}\n{dmg}")


if __name__ == "__main__":
    main()
