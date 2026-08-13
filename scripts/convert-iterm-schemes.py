#!/usr/bin/env python3
"""Convert iTerm2-Color-Schemes .itermcolors plists into Sleipnir themes.json.

Usage:
  python3 scripts/convert-iterm-schemes.py \\
      /path/to/iTerm2-Color-Schemes/schemes \\
      resources/themes.json

Source catalog: https://github.com/mbadolato/iTerm2-Color-Schemes (MIT).
The checked-in `resources/themes.json` was generated from that catalog
(601 complete 16-color palettes). Re-run this script to refresh.
"""
from __future__ import annotations

import glob
import json
import os
import plistlib
import sys

# .itermcolors plist key -> CustomPalette field
COLOR_KEYS = {
    "Background Color": "background",
    "Foreground Color": "foreground",
    "Bold Color": "bright_foreground",
    "Cursor Color": "cursor",
    "Selection Color": "selection",
}


def rgb_to_hex(d):
    """Extract #rrggbb from an itermcolors color dict (0..1 floats or 0..255)."""
    if not isinstance(d, dict):
        return None
    comps = []
    for suffix in ("Red Component", "Green Component", "Blue Component"):
        v = d.get(suffix)
        if v is None:
            return None
        comps.append(v)
    r, g, b = comps
    if max(r, g, b) > 1.0:
        r, g, b = r / 255.0, g / 255.0, b / 255.0
    return "#%02x%02x%02x" % (round(r * 255), round(g * 255), round(b * 255))


def convert(path):
    with open(path, "rb") as f:
        plist = plistlib.loads(f.read())
    out = {}
    for plist_key, field in COLOR_KEYS.items():
        if plist_key in plist:
            hexv = rgb_to_hex(plist[plist_key])
            if hexv:
                out[field] = hexv
    ansi = []
    for i in range(16):
        if f"Ansi {i} Color" in plist:
            hexv = rgb_to_hex(plist[f"Ansi {i} Color"])
            ansi.append(hexv)
        else:
            ansi.append(None)
    while ansi and ansi[-1] is None:
        ansi.pop()
    if len(ansi) < 16:
        return None
    out["ansi"] = [a for a in ansi if a is not None][:16]
    if len(out["ansi"]) != 16:
        return None
    return out


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(
            "usage: convert-iterm-schemes.py <schemes_dir> <out.json>",
            file=sys.stderr,
        )
        return 2
    src, out_path = argv[1], argv[2]
    themes = {}
    skipped = []
    for path in sorted(glob.glob(os.path.join(src, "*.itermcolors"))):
        name = os.path.basename(path)[: -len(".itermcolors")]
        try:
            palette = convert(path)
        except Exception as err:  # noqa: BLE001
            skipped.append((name, str(err)))
            continue
        if palette is None:
            skipped.append((name, "incomplete ansi"))
            continue
        themes[name] = palette

    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    with open(out_path, "w") as f:
        json.dump(themes, f, indent=1, ensure_ascii=False)

    print(f"converted {len(themes)} themes -> {out_path}")
    print(f"skipped {len(skipped)}: {skipped[:5]}")
    print(f"size: {os.path.getsize(out_path)} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
