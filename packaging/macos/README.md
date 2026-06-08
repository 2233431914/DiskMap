# macOS Packaging

DiskMap ships as a standard macOS app bundle generated from the release
binary. The packaging script is intentionally dependency-light: it uses Cargo
plus Apple command-line tools already present with Xcode Command Line Tools.

## Local App Bundle

```bash
scripts/package-macos.sh
```

Outputs:

- `target/dist/DiskMap.app`
- `target/dist/DiskMap-<version>-macos-<arch>.zip`

The default signing identity is `-`, which creates an ad-hoc signature. This
is enough for local testing and for preserving the bundle's code signature
shape, but it is not suitable for public distribution.

## Developer ID Signing

```bash
scripts/package-macos.sh \
  --bundle-id com.example.diskmap \
  --identity "Developer ID Application: Your Name (TEAMID)"
```

The script signs the app with hardened runtime enabled and verifies the bundle
with `codesign --verify --deep --strict`.

## Notarization

Store credentials once:

```bash
xcrun notarytool store-credentials diskmap-notary \
  --apple-id you@example.com \
  --team-id TEAMID \
  --password app-specific-password
```

Then package, submit, wait, staple, validate, and rebuild the zip:

```bash
NOTARYTOOL_PROFILE=diskmap-notary \
scripts/package-macos.sh \
  --bundle-id com.example.diskmap \
  --identity "Developer ID Application: Your Name (TEAMID)" \
  --notarize
```

As an alternative to `NOTARYTOOL_PROFILE`, set `APPLE_ID`, `APPLE_TEAM_ID`, and
`APPLE_APP_PASSWORD` in the environment.

## DMG

```bash
scripts/package-macos.sh --dmg
```

This creates a simple compressed DMG with `DiskMap.app` and an Applications
symlink. The zip remains the canonical notarization input because Apple's
`notarytool` flow is reliable with zipped app bundles.

## Cross-Arch Builds

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/package-macos.sh --target aarch64-apple-darwin
scripts/package-macos.sh --target x86_64-apple-darwin
```

Universal binary packaging is intentionally not automated yet. Build both
targets and combine the binaries with `lipo` only when distribution requires
one universal app artifact.
