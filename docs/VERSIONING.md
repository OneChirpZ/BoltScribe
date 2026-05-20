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
