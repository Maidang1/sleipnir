#!/usr/bin/env python3
"""Reject Windows release executables that would open an extra console window.

Usage: python scripts/tests/check-windows-subsystem.py path/to/sleipnir.exe
"""

import struct
import sys
from pathlib import Path


def check_gui_subsystem(path):
    with path.open("rb") as exe:
        if exe.read(2) != b"MZ":
            raise ValueError("missing DOS header")
        exe.seek(0x3C)
        pe_offset = struct.unpack("<I", exe.read(4))[0]
        exe.seek(pe_offset)
        if exe.read(4) != b"PE\0\0":
            raise ValueError("missing PE signature")
        # The optional header follows the PE signature and 20-byte COFF header.
        optional_header = pe_offset + 24
        exe.seek(optional_header)
        if struct.unpack("<H", exe.read(2))[0] not in (0x10B, 0x20B):
            raise ValueError("unsupported PE optional header")
        # Subsystem is at offset 68 in both PE32 and PE32+ optional headers.
        exe.seek(optional_header + 68)
        subsystem = struct.unpack("<H", exe.read(2))[0]
        if subsystem != 2:  # IMAGE_SUBSYSTEM_WINDOWS_GUI
            raise ValueError(f"expected Windows GUI subsystem (2), got {subsystem}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"Usage: {sys.argv[0]} path/to/sleipnir.exe")
    path = Path(sys.argv[1])
    try:
        check_gui_subsystem(path)
    except (OSError, ValueError, struct.error) as error:
        raise SystemExit(f"FAIL: {path}: {error}")
    print(f"PASS: {path}: Windows GUI subsystem (2)")
