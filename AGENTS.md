# Local Agent Notes

- For any change, consider macOS and Windows compatibility. Use shared code for common behavior; for platform-specific work, leave a clear interface for the other platform.
- Version numbers must stay in sync across `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`.
- Versions must be valid semver. Before packaging or publishing, run `npm run version:check`.
