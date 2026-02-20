# Agent Orchestration

This directory defines the operating model for multi-agent development in this repository.

## Model Stack

- OpenAI: `GPT-5.3 Codex`
- Gemini: `gemini-3.1pro`

Global policy: only `GPT-5.3 Codex` and `gemini-3.1pro` are used.

## Core Roles

- `Planner/Orchestrator` (`GPT-5.3 Codex`): decomposes requests, sets risk level, picks execution path.
- `Code Specialist` (`GPT-5.3 Codex` + `gemini-3.1pro` in parallel): implements complex Rust changes and deep debugging.
- `Builder` (`GPT-5.3 Codex` + `gemini-3.1pro` in parallel): handles straightforward implementation, docs, and polish.
- `Verifier` (`GPT-5.3 Codex`): test strategy, failure triage, and cross-check.
- `Security Gate` (`GPT-5.3 Codex`): final review for security-sensitive changes.

## Documents

- `docs/agent-orchestration/architecture.md`
- `docs/agent-orchestration/routing-rules.md`
- `docs/agent-orchestration/policies.md`
- `docs/agent-orchestration/prompt-templates.md`
- `docs/agent-orchestration/runbook-fallback.md`
- `docs/agent-orchestration/two-week-rollout.md`
