#!/bin/bash
# Package a native binary and a self-contained sign-in runtime.
set -euo pipefail
project="$(cd -- "$(dirname -- "$0")/.." && pwd)"
workspace="$(cd -- "$project/../.." && pwd)"
tools="${CLASSROOM_RUNTIME_DIR:-$project/desktop/runtime}"
if [[ -z "${CLASSROOM_RUNTIME_DIR:-}" && ! -x "$tools/venv/bin/python" ]]; then
  tools="$workspace/work/notebooklm-test"
fi
binary="${1:-${CARGO_TARGET_DIR:-$project/target}/release/classroom-desktop}"
app="${2:-$project/dist/Classroom for Mac.app}"
parent="$(dirname -- "$app")"
mkdir -p -- "$parent"
staging="$(mktemp -d "$parent/.classroom-bundle.XXXXXX")"
candidate="$staging/Classroom for Mac.app"
previous="$staging/previous.app"
cleanup() {
  if [[ -d "$previous" && ! -e "$app" ]]; then
    mv -- "$previous" "$app" || {
      printf 'Previous app retained at %s\n' "$previous" >&2
      return
    }
  fi
  rm -rf -- "$staging"
}
trap cleanup EXIT
contents="$candidate/Contents"
resources="$contents/Resources"

if [[ ! -x "$binary" || ! -x "$tools/venv/bin/python" ]]; then
  printf '%s\n' 'Expected a built desktop binary and the installed sign-in runtime.' >&2
  exit 2
fi
if [[ -e "$app" || -L "$app" ]]; then
  if [[ ! -d "$app" || -L "$app" ]] ||
    [[ "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist" 2>/dev/null)" != "local.classroom-rust.desktop" ]] ||
    [[ "$(plutil -extract CFBundleExecutable raw "$app/Contents/Info.plist" 2>/dev/null)" != "classroom-desktop" ]]; then
    printf 'Expected a Classroom for Mac app destination at %s\n' "$app" >&2
    exit 2
  fi
fi
mkdir -p "$contents/MacOS" "$resources/Browsers"
install -m 755 "$binary" "$contents/MacOS/classroom-desktop"
install -m 644 "$project/desktop/google_sign_in.py" "$resources/google_sign_in.py"
install -m 644 "$project/desktop/session_keychain.py" "$resources/session_keychain.py"
install -m 644 "$project/desktop/drive_upload.py" "$resources/drive_upload.py"
install -m 644 "$project/assets/AppIcon.icns" "$resources/AppIcon.icns"
install -m 644 "$project/assets/fonts/OFL.txt" "$resources/Google-Sans-Flex-OFL.txt"

python_base="$("$tools/venv/bin/python" -I -c 'import sys; print(sys.base_prefix)')"
ditto "$python_base" "$resources/Runtime"
source_packages="$tools/venv/lib/python3.12/site-packages"
packages="$resources/Runtime/lib/python3.12/site-packages"
mkdir -p "$packages"
for package in playwright greenlet gpsoauth requests urllib3 certifi idna charset_normalizer Cryptodome; do
  ditto "$source_packages/$package" "$packages/$package"
done
for metadata in "$source_packages"/*.dist-info; do
  case "$(basename -- "$metadata")" in
    playwright-*|greenlet-*|gpsoauth-*|requests-*|urllib3-*|certifi-*|idna-*|charset_normalizer-*|pycryptodomex-*)
      ditto "$metadata" "$packages/$(basename -- "$metadata")" ;;
  esac
done
for browser in "$tools/browsers"/chromium-[0-9]*; do
  ditto "$browser" "$resources/Browsers/$(basename -- "$browser")"
done

cat > "$contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>classroom-desktop</string>
  <key>CFBundleIdentifier</key><string>local.classroom-rust.desktop</string>
  <key>CFBundleName</key><string>Classroom for Mac</string>
  <key>CFBundleDisplayName</key><string>Classroom for Mac</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon.icns</string>
  <key>CFBundleShortVersionString</key><string>0.78.0</string>
  <key>CFBundleVersion</key><string>78</string>
  <key>NSMicrophoneUsageDescription</key><string>Record audio when you choose to add a recording to an assignment.</string>
  <key>NSCameraUsageDescription</key><string>Record video when you choose to add a recording to an assignment.</string>
  <key>NSScreenCaptureUsageDescription</key><string>Record the screen area you select for an assignment.</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.education</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$contents/Info.plist"
"$resources/Runtime/bin/python3.12" -I -B -c 'import gpsoauth, playwright.sync_api'
codesign --force --sign - "$candidate"
codesign --verify --strict "$candidate"
if [[ -d "$app" ]]; then
  mv -- "$app" "$previous"
fi
mv -- "$candidate" "$app"
printf 'Packaged %s\n' "$app"
