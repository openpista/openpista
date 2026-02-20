# Security Advisory Exceptions

This file tracks temporary `cargo audit` exceptions and the mitigation applied.

## RUSTSEC-2023-0071 (`rsa` timing sidechannel)

- Status: temporarily ignored in `.cargo/audit.toml`
- Source in lockfile: `sqlx-mysql` transitive dependency
- Applicability in this workspace: runtime is configured for SQLite only (`sqlx` with `sqlite`/`migrate` features, no MySQL driver usage)
- Upstream state: no fixed upgrade published for this advisory
- Mitigation in place:
  - MySQL paths are not used in code/runtime configuration
  - CI still runs `cargo audit`; only this single advisory is ignored
  - Re-evaluate and remove ignore once upstream publishes a fix
