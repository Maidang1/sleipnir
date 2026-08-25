#!/usr/bin/env python3
"""Create and optionally sign Sleipnir's canonical update manifest."""

import argparse
import base64
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--dmg", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--signature", type=Path, required=True)
    args = parser.parse_args()

    payload = {
        "schema_version": 1,
        "version": args.version,
        "tag": f"v{args.version}",
        "artifact": args.dmg.name,
        "size": args.dmg.stat().st_size,
        "sha256": hashlib.sha256(args.dmg.read_bytes()).hexdigest(),
        "bundle_id": "com.maidang1.sleipnir",
        "minimum_macos": "14.0",
        "minimum_updater_schema": 1,
    }
    args.output.write_bytes(json.dumps(payload, separators=(",", ":"), ensure_ascii=True).encode())
    args.signature.unlink(missing_ok=True)

    encoded_key = os.environ.get("SLEIPNIR_UPDATE_SIGNING_KEY")
    if not encoded_key:
        print("SLEIPNIR_UPDATE_SIGNING_KEY is unset; wrote unsigned local manifest")
        return 0

    with tempfile.NamedTemporaryFile(mode="wb", delete=False) as key_file:
        key_path = Path(key_file.name)
        key_file.write(base64.b64decode(encoded_key, validate=True))
    try:
        key_path.chmod(0o600)
        subprocess.run(
            ["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(key_path), "-in", str(args.output), "-out", str(args.signature)],
            check=True,
        )
    finally:
        key_path.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
