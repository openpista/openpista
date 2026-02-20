# Changelog

All notable changes to this project will be documented in this file.
The format is based on Keep a Changelog.

## v0.1.0 - TBD

### Added
- [cli] Added `start`, `run -e`, and `repl` command flows for daemon startup, one-shot execution, and interactive sessions.
- [agent] Added an LLM-driven runtime with tool registry integration and SQLite-backed memory.
- [skills] Added workspace-based skill context loading for agent requests.
- [gateway] Added a QUIC server bootstrap path for in-process event handling.
- [channels] Added CLI and optional Telegram channel adapters for channel event ingestion.
- [tools] Added working `browser.navigate`, `browser.click`, `browser.type`, and `browser.screenshot` tools, plus `screen.capture` output support.
- [tools] Added `container.run` for isolated per-task Docker execution via `bollard`.
- [tools] Added optional task-scoped short-lived credential injection for `container.run` using in-memory `/run/secrets` upload and automatic credential disposal.
- [workspace] Added a sample configuration file (`config.example.toml`) covering gateway, agent, channel, database, and skills settings.

### Changed
- None.

### Fixed
- None.

### Known Limitations
- [channels] Advanced multi-channel routing is out of scope for v0.1.0.

When preparing a new release, add `## vX.Y.Z - TBD` at the top of this file.
On tag/release day, replace `TBD` with `YYYY-MM-DD`.
