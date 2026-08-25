#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

printf 'test dmg bytes' > "${TMP}/Sleipnir-9.8.7-macos.dmg"
openssl genpkey -algorithm Ed25519 -out "${TMP}/private.pem" >/dev/null 2>&1
openssl pkey -in "${TMP}/private.pem" -pubout -out "${TMP}/public.pem" >/dev/null 2>&1
SLEIPNIR_UPDATE_SIGNING_KEY="$(base64 < "${TMP}/private.pem" | tr -d '\n')"
export SLEIPNIR_UPDATE_SIGNING_KEY

"${ROOT}/scripts/sign-update-manifest.py" \
  --version 9.8.7 \
  --dmg "${TMP}/Sleipnir-9.8.7-macos.dmg" \
  --output "${TMP}/sleipnir-update-v1.json" \
  --signature "${TMP}/sleipnir-update-v1.json.sig" \
  --public-key "${TMP}/public.pem"

openssl pkeyutl -verify -rawin -pubin \
  -inkey "${TMP}/public.pem" \
  -in "${TMP}/sleipnir-update-v1.json" \
  -sigfile "${TMP}/sleipnir-update-v1.json.sig"

python3 - "${TMP}" <<'PY'
import hashlib
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
dmg = root / "Sleipnir-9.8.7-macos.dmg"
manifest = json.loads((root / "sleipnir-update-v1.json").read_text())
assert manifest["version"] == "9.8.7"
assert manifest["tag"] == "v9.8.7"
assert manifest["artifact"] == dmg.name
assert manifest["size"] == dmg.stat().st_size
assert manifest["sha256"] == hashlib.sha256(dmg.read_bytes()).hexdigest()
assert manifest["bundle_id"] == "com.maidang1.sleipnir"
PY

grep -q 'sleipnir-update-helper' "${ROOT}/scripts/make-app.sh"
grep -q 'SLEIPNIR_UPDATE_SIGNING_KEY' "${ROOT}/.github/workflows/build-and-release.yml"
grep -q 'sleipnir-update-v1.json.sig' "${ROOT}/.github/workflows/build-and-release.yml"
echo "transactional update release metadata: PASS"
