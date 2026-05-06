#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <FileShare.app path> <output dmg path>" >&2
  exit 1
fi

app_path="$1"
output_path="$2"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
repair_tool="$repo_root/scripts/macos/损坏修复"
staging_dir="$(mktemp -d)"

cleanup() {
  rm -rf "$staging_dir"
}
trap cleanup EXIT

if [ ! -d "$app_path" ]; then
  echo "App bundle not found: $app_path" >&2
  exit 1
fi

if [ ! -f "$repair_tool" ]; then
  echo "Repair tool not found: $repair_tool" >&2
  exit 1
fi

mkdir -p "$(dirname "$output_path")"
cp -R "$app_path" "$staging_dir/FileShare.app"
cp "$repair_tool" "$staging_dir/损坏修复"
chmod +x "$staging_dir/损坏修复"
ln -s /Applications "$staging_dir/Applications"

hdiutil create \
  -volname "FileShare" \
  -srcfolder "$staging_dir" \
  -ov \
  -format UDZO \
  "$output_path"
