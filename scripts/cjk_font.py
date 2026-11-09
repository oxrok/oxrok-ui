#!/usr/bin/env python3
"""Builds the bundled CJK font subsets from the translation catalog.

Takes the Noto Sans SC variable font, pins three weights matching Inter
(Regular 400, Medium 500, Bold 700) and keeps only the CJK characters
used in src/l10n.rs, writing assets/fonts/NotoSansSC-<Weight>-subset.ttf.

Rerun after adding Chinese strings (`make cjk-font`); the l10n font test
fails when the catalog uses a character the subset lacks.

Usage: scripts/cjk_font.py NotoSansSC[wght].ttf
(needs fontTools: `uv run --with fonttools python3 scripts/cjk_font.py …`)
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "src" / "l10n.rs"
OUT = ROOT / "assets" / "fonts"
WEIGHTS = {"Regular": 400, "Medium": 500, "Bold": 700}


def catalog_chars() -> str:
    # CJK scripts, punctuation and full-width forms; Inter covers the rest.
    text = CATALOG.read_text(encoding="utf-8")
    return "".join(sorted({c for c in text if ord(c) >= 0x2E80}))


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    chars = catalog_chars()
    print(f"{len(chars)} CJK characters in {CATALOG.relative_to(ROOT)}")
    for name, wght in WEIGHTS.items():
        font = instancer.instantiateVariableFont(TTFont(sys.argv[1]), {"wght": wght})
        options = subset.Options()
        options.layout_features = ["*"]
        options.name_IDs = ["*"]
        options.notdef_outline = True
        sub = subset.Subsetter(options)
        sub.populate(text=chars)
        sub.subset(font)
        path = OUT / f"NotoSansSC-{name}-subset.ttf"
        font.save(path)
        print(f"wrote {path.relative_to(ROOT)} ({path.stat().st_size // 1024} KiB)")


if __name__ == "__main__":
    main()
