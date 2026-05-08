#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 5 ] || [ "$#" -gt 6 ]; then
  echo "Usage: $0 <platform> <version> <update asset path> <signature path> <output json path> [notes]" >&2
  exit 1
fi

platform="$1"
version="$2"
asset_path="$3"
signature_path="$4"
output_path="$5"
notes_text="${6:-}"
asset_name="$(basename "$asset_path")"
download_url="https://github.com/tri5m/file-share/releases/latest/download/$asset_name"

python3 - "$platform" "$version" "$download_url" "$signature_path" "$output_path" "$notes_text" <<'PY'
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

platform, version, url, signature_path, output_path = sys.argv[1:6]
notes_text = sys.argv[6].strip() if len(sys.argv) > 6 else ""
signature = Path(signature_path).read_text(encoding="utf-8").strip()

notes = notes_text or f"""FileShare {version}

🔄 更新内容：
请访问 GitHub 发布页面查看完整的更新日志和新功能说明。

📦 下载地址：
https://github.com/tri5m/file-share/releases/tag/v{version}
"""

payload = {
    "version": version,
    "notes": notes.strip(),
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
