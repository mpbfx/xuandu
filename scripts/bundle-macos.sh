#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "bundle:mac 只能在 macOS 上运行。" >&2
  exit 1
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_path="$root_dir/src-tauri/target/release/bundle/macos/选读.app"
config_path="$root_dir/src-tauri/tauri.conf.json"
bundle_id="com.mpbfx.xuandu"
version="$(sed -nE 's/^[[:space:]]*"version":[[:space:]]*"([^"]+)".*/\1/p' "$config_path" | head -n 1)"

case "$(uname -m)" in
  arm64) bundle_arch="aarch64" ;;
  x86_64) bundle_arch="x64" ;;
  *)
    echo "不支持的 macOS 架构：$(uname -m)" >&2
    exit 1
    ;;
esac

if [[ -z "$version" || ! -d "$app_path" ]]; then
  echo "未找到已构建的选读.app；请先运行 Tauri 构建。" >&2
  exit 1
fi

# Without this explicit designated requirement, an ad-hoc signature uses the
# executable hash as its identity. macOS then treats every rebuild as a new
# client and drops the Accessibility grant.
codesign --force --deep --sign - --identifier "$bundle_id" \
  -r="designated => identifier \"$bundle_id\"" "$app_path"
codesign --verify --deep --strict --verbose=2 "$app_path"

dmg_dir="$root_dir/src-tauri/target/release/bundle/dmg"
dmg_path="$dmg_dir/选读_${version}_${bundle_arch}.dmg"
staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/xuandu-dmg.XXXXXX")"
trap 'rm -rf "$staging_dir"' EXIT

mkdir -p "$dmg_dir"
ditto "$app_path" "$staging_dir/选读.app"
hdiutil create -volname "选读" -srcfolder "$staging_dir" -format UDZO -ov "$dmg_path"

echo "已生成：$app_path"
echo "已生成：$dmg_path"
