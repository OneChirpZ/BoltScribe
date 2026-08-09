# GitHub Repository Profile

Use this when creating the public GitHub repository. Do not include personal API keys, local machine paths, or private screenshots in the repository metadata.

## Repository Name

`boltscribe`

## Short Description

Local-first macOS and Windows voice input with ASR transcription, LLM correction, global hotkeys, and a Tauri tray workflow.

## Longer Description

BoltScribe is a macOS and Windows dictation app that records speech from a global hotkey or tray action, transcribes it with ASR, optionally corrects it through OpenAI-compatible LLM providers, and pastes the final text back into the active app. It includes model presets, cross-provider race mode, local history, configurable local data storage, retention controls, persistent usage stats, and a floating recording capsule.

## Suggested Topics

```text
macos
windows
tauri
react
rust
typescript
voice-input
dictation
speech-to-text
asr
llm
openai-compatible
menu-bar
tray
productivity
accessibility
volcengine
```

## Suggested Social Preview

Use `docs/assets/boltscribe-workflow-en.svg` or `docs/assets/boltscribe-workflow-zh.svg`, depending on the audience. Avoid screenshots that expose real history records, API keys, usernames, local paths, or private app content.

## Pre-Publish Checklist

- Add a project `LICENSE` file selected by the project owner.
- Publish from a sanitized history or an orphan release branch if personal metadata appears in the existing Git history.
- Confirm `config.default.json` and `config.example.json` contain empty credential fields.
- Confirm `.codex/`, local configs, generated bundles, recordings, logs, and signing certificates are not tracked.
- Run a final history scan for personal paths, usernames, author emails, API keys, tokens, and `.env` files before pushing.
