# Local Agent Notes

- For any change, consider macOS and Windows compatibility. Use shared code for common behavior; for platform-specific work, leave a clear interface for the other platform.
- Version numbers must stay in sync across `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`.
- Versions must be valid semver. Before packaging or publishing, run `npm run version:check`.
- Build public macOS packages with `npm run release:macos`; it removes local home paths from Rust artifacts.

## Development Workflow

- Keep `main` stable, runnable, and release-ready.
- Before any public push or tag, use the repository's GitHub noreply identity and scan committed content and Git metadata for secrets, personal email, usernames, and local paths.
- Use short-lived local branches for non-trivial work, named `codex/<task-name>` by default. Tiny, low-risk fixes may go directly on `main`.
- Before branch switching, merging, rebasing, restoring, or creating worktrees, check `git status --short --branch`; also check `git worktree list --porcelain` when worktrees are involved.
- Prefer squash-merging ordinary task branches into `main` after focused verification. Use normal merges for experiment lines whose history matters.
- After a branch has been successfully merged into `main`, archive the local branch by renaming it under `archived/<branch-name>` instead of leaving it under its original `codex/...`, `feat/...`, or other active namespace.
- Keep branches local unless the user asks to push, publish, create a PR, or make a remote backup. Release from `main` after version sync and packaging checks.
