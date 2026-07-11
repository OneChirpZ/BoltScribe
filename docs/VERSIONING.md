# Versioning

BoltScribe uses semantic versioning for public releases.

Keep these files on the same version before building a release:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`

Run this check before packaging:

```bash
npm run version:check
```

Release tags should use the same version with a `v` prefix, for example:

```text
v1.1.0
```

Release DMG files should include the same version in the filename, for example:

```text
BoltScribe_1.1.0_aarch64.dmg
```

Windows NSIS installers should follow the corresponding Tauri-generated name:

```text
BoltScribe_1.1.0_x64-setup.exe
```

Before publishing a GitHub release, verify the generated macOS app and DMG:

```bash
codesign --verify --deep --strict src-tauri/target/release/bundle/macos/BoltScribe.app
hdiutil verify src-tauri/target/release/bundle/dmg/BoltScribe_<version>_aarch64.dmg
shasum -a 256 src-tauri/target/release/bundle/dmg/BoltScribe_<version>_aarch64.dmg
```

Build the Windows installer on the native Windows runner with the
`Build Windows release` workflow, then verify its SHA-256 digest before upload.

If notarization environment variables are not configured, call that out in the release notes.
