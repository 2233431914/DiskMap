#!/usr/bin/env bash
set -euo pipefail

APP_NAME="DiskMap"
BINARY_NAME="disk-map"
APP_EXECUTABLE="DiskMap"
DEFAULT_BUNDLE_ID="com.ivan.diskmap"
MIN_MACOS_VERSION="${MACOSX_DEPLOYMENT_TARGET:-13.0}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${APP_VERSION:-$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)}"
BUILD_VERSION="${BUILD_NUMBER:-$VERSION}"
BUNDLE_ID="${BUNDLE_ID:-$DEFAULT_BUNDLE_ID}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:--}"
OUTPUT_ROOT="${OUTPUT_ROOT:-target/dist}"
TARGET_TRIPLE="${TARGET_TRIPLE:-}"
ICON_FILE="${ICON_FILE:-packaging/macos/DiskMap.icns}"

DO_BUILD=1
DO_SIGN=1
DO_ZIP=1
DO_DMG=0
DO_NOTARIZE=0

usage() {
    cat <<USAGE
Usage: scripts/package-macos.sh [options]

Build a release binary and package it as DiskMap.app.

Options:
  --bundle-id ID        CFBundleIdentifier (default: ${DEFAULT_BUNDLE_ID})
  --identity ID         codesign identity (default: ad-hoc "-")
  --target TRIPLE      Cargo target triple, e.g. aarch64-apple-darwin
  --output DIR         Output directory (default: target/dist)
  --icon FILE          Optional .icns icon path (default: packaging/macos/DiskMap.icns)
  --skip-build         Reuse an existing release binary
  --skip-sign          Do not run codesign
  --no-zip             Do not create the release zip
  --dmg                Also create a simple DMG with an Applications symlink
  --notarize           Submit the zip with xcrun notarytool, then staple the app
  -h, --help           Show this help

Environment:
  APP_VERSION, BUILD_NUMBER, BUNDLE_ID, CODESIGN_IDENTITY, OUTPUT_ROOT,
  TARGET_TRIPLE, ICON_FILE, MACOSX_DEPLOYMENT_TARGET.

Notarization credentials:
  Use NOTARYTOOL_PROFILE for a stored keychain profile, or set APPLE_ID,
  APPLE_TEAM_ID, and APPLE_APP_PASSWORD.
USAGE
}

die() {
    echo "error: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bundle-id)
            [[ $# -ge 2 ]] || die "--bundle-id requires a value"
            BUNDLE_ID="$2"
            shift 2
            ;;
        --identity)
            [[ $# -ge 2 ]] || die "--identity requires a value"
            CODESIGN_IDENTITY="$2"
            shift 2
            ;;
        --target)
            [[ $# -ge 2 ]] || die "--target requires a value"
            TARGET_TRIPLE="$2"
            shift 2
            ;;
        --output)
            [[ $# -ge 2 ]] || die "--output requires a value"
            OUTPUT_ROOT="$2"
            shift 2
            ;;
        --icon)
            [[ $# -ge 2 ]] || die "--icon requires a value"
            ICON_FILE="$2"
            shift 2
            ;;
        --skip-build)
            DO_BUILD=0
            shift
            ;;
        --skip-sign)
            DO_SIGN=0
            shift
            ;;
        --no-zip)
            DO_ZIP=0
            shift
            ;;
        --dmg)
            DO_DMG=1
            shift
            ;;
        --notarize)
            DO_NOTARIZE=1
            DO_ZIP=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || die "macOS packaging must run on macOS"

require_command cargo
require_command plutil
require_command codesign
require_command ditto

export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS_VERSION"

if [[ "$DO_BUILD" -eq 1 ]]; then
    cargo_args=(build --release --bin "$BINARY_NAME")
    if [[ -n "$TARGET_TRIPLE" ]]; then
        cargo_args+=(--target "$TARGET_TRIPLE")
    fi
    cargo "${cargo_args[@]}"
fi

if [[ -n "$TARGET_TRIPLE" ]]; then
    RELEASE_DIR="target/$TARGET_TRIPLE/release"
    ARCH_LABEL="$TARGET_TRIPLE"
else
    RELEASE_DIR="target/release"
    ARCH_LABEL="$(uname -m)"
fi

BINARY_PATH="$RELEASE_DIR/$BINARY_NAME"
[[ -x "$BINARY_PATH" ]] || die "release binary not found: $BINARY_PATH"

mkdir -p "$OUTPUT_ROOT"
OUTPUT_ROOT="$(cd "$OUTPUT_ROOT" && pwd)"
APP_DIR="$OUTPUT_ROOT/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLIST_PATH="$CONTENTS_DIR/Info.plist"
ZIP_PATH="$OUTPUT_ROOT/${APP_NAME}-${VERSION}-macos-${ARCH_LABEL}.zip"
DMG_PATH="$OUTPUT_ROOT/${APP_NAME}-${VERSION}-macos-${ARCH_LABEL}.dmg"

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$BINARY_PATH" "$MACOS_DIR/$APP_EXECUTABLE"
chmod 755 "$MACOS_DIR/$APP_EXECUTABLE"

ICON_PLIST_ENTRY=""
if [[ -f "$ICON_FILE" ]]; then
    cp "$ICON_FILE" "$RESOURCES_DIR/$APP_NAME.icns"
    ICON_PLIST_ENTRY="    <key>CFBundleIconFile</key>
    <string>$APP_NAME</string>"
fi

cat >"$PLIST_PATH" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$APP_EXECUTABLE</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$BUILD_VERSION</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key>
    <string>$MIN_MACOS_VERSION</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
$ICON_PLIST_ENTRY
</dict>
</plist>
PLIST

printf "APPL????" >"$CONTENTS_DIR/PkgInfo"
plutil -lint "$PLIST_PATH" >/dev/null

if [[ "$DO_SIGN" -eq 1 ]]; then
    sign_args=(--force --options runtime --sign "$CODESIGN_IDENTITY")
    if [[ "$CODESIGN_IDENTITY" != "-" ]]; then
        sign_args+=(--timestamp)
    fi
    codesign "${sign_args[@]}" "$APP_DIR"
    codesign --verify --deep --strict --verbose=2 "$APP_DIR" >/dev/null
else
    echo "Skipping codesign"
fi

create_zip() {
    rm -f "$ZIP_PATH"
    (cd "$OUTPUT_ROOT" && ditto -c -k --keepParent "$APP_NAME.app" "$(basename "$ZIP_PATH")")
}

if [[ "$DO_ZIP" -eq 1 ]]; then
    create_zip
fi

if [[ "$DO_NOTARIZE" -eq 1 ]]; then
    [[ "$DO_SIGN" -eq 1 ]] || die "--notarize requires signing"
    [[ "$CODESIGN_IDENTITY" != "-" ]] || die "--notarize requires a Developer ID signing identity"
    require_command xcrun

    notary_args=()
    if [[ -n "${NOTARYTOOL_PROFILE:-}" ]]; then
        notary_args+=(--keychain-profile "$NOTARYTOOL_PROFILE")
    elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
        notary_args+=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD")
    else
        die "set NOTARYTOOL_PROFILE or APPLE_ID + APPLE_TEAM_ID + APPLE_APP_PASSWORD for notarization"
    fi

    xcrun notarytool submit "$ZIP_PATH" --wait "${notary_args[@]}"
    xcrun stapler staple "$APP_DIR"
    xcrun stapler validate "$APP_DIR"
    create_zip
fi

if [[ "$DO_DMG" -eq 1 ]]; then
    require_command hdiutil
    DMG_STAGING="$(mktemp -d "$OUTPUT_ROOT/dmg-staging.XXXXXX")"
    cleanup_dmg_staging() {
        rm -rf "$DMG_STAGING"
    }
    trap cleanup_dmg_staging EXIT
    cp -R "$APP_DIR" "$DMG_STAGING/"
    ln -s /Applications "$DMG_STAGING/Applications"
    rm -f "$DMG_PATH"
    hdiutil create -volname "$APP_NAME" -srcfolder "$DMG_STAGING" -ov -format UDZO "$DMG_PATH" >/dev/null
fi

echo "App bundle: $APP_DIR"
if [[ "$DO_ZIP" -eq 1 ]]; then
    echo "Zip: $ZIP_PATH"
fi
if [[ "$DO_DMG" -eq 1 ]]; then
    echo "DMG: $DMG_PATH"
fi
