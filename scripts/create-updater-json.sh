#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo "Usage: $0 <platform> <version> <update asset path> <signature path> <output json path>" >&2
  exit 1
fi

platform="$1"
version="$2"
asset_path="$3"
signature_path="$4"
output_path="$5"
asset_name="$(basename "$asset_path")"
download_url="https://github.com/tri5m/file-share/releases/latest/download/$asset_name"

python3 - "$platform" "$version" "$download_url" "$signature_path" "$output_path" <<'PY'
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

platform, version, url, signature_path, output_path = sys.argv[1:]
signature = Path(signature_path).read_text(encoding="utf-8").strip()
payload = {
    "version": version,
    "notes": f"FileShare {version}",
    "pub_date": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
    "platforms": {
        platform: {
            "signature": signature,
            "url": url,
        }
    },
}
Path(output_path).parent.mkdir(parents=True, exist_ok=True)
Path(output_path).write_text(
    json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY
