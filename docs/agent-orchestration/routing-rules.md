# Routing Rules

This file defines deterministic routing between roles and models.

## Roles and Default Models

| Role | Default Model | Parallel Sub-agent | Purpose |
|---|---|---|---|
| Planner/Orchestrator | OpenAI GPT-5.3 Codex | None | Task decomposition, sequencing, and risk decisions |
| Code Specialist | OpenAI GPT-5.3 Codex + gemini-3.1pro | Parallel pair | Complex Rust implementation and debugging |
| Builder | OpenAI GPT-5.3 Codex + gemini-3.1pro | Parallel pair | Medium/low complexity implementation and documentation |
| Verifier | OpenAI GPT-5.3 Codex | None | Test design and root-cause analysis |
| Security Gate | OpenAI GPT-5.3 Codex | None | Security boundary and release-risk review |

## Universal Model Policy

Only these models are allowed in orchestration:

- `GPT-5.3 Codex`
- `gemini-3.1pro`

Planning lane rule:

- Plan uses `GPT-5.3 Codex` only.

Build/implementation lane rule:

- Build and implementation always run in parallel with `GPT-5.3 Codex` + `gemini-3.1pro`.

Selection rule for parallel build outputs:

1. Prefer output that satisfies all acceptance criteria with smaller diff.
2. If both satisfy criteria, prefer the one with clearer verification evidence.
3. If outputs conflict on architecture direction, escalate to human decision.

## Scoring

Compute `Score = Complexity + Risk + BlastRadius`.

- Complexity: 1..5
- Risk: 1..5
- BlastRadius: 1..5

## Dispatch

- `Score 3..6`: `Builder` (parallel `GPT-5.3 Codex` + `gemini-3.1pro`)
- `Score 7..10`: `Code Specialist` (parallel `GPT-5.3 Codex` + `gemini-3.1pro`)
- `Score 11..15`: `Code Specialist` (parallel `GPT-5.3 Codex` + `gemini-3.1pro`) + `Planner` review (`GPT-5.3 Codex`)

If one parallel lane is unavailable:

- `Score 3..15`: continue with the available lane and require verifier notes for the missing lane.

## Mandatory Escalation

Always require `Security Gate` (`GPT-5.3 Codex`) when change touches:

- command execution boundaries (`system.run`, shell arguments, process spawning)
- auth/session/token handling
- gateway transport/session routing
- release scripts, CI permissions, secrets-related configuration

## Retry and Fallback

1. First failure: retry once on same model with tighter prompt constraints.
2. Second failure:
   - keep `Codex + gemini-3.1pro` pair, tighten prompt and shrink change scope
3. Third failure or unclear root cause: escalate to `Planner` (`GPT-5.3 Codex`) for plan rewrite.
4. If one model lane is unavailable, continue with the healthy lane and mark reduced redundancy in notes.

## Merge Conditions

- Required checks pass (test/lint/security scan).
- High-risk changes include `Security Gate` notes.
- PR includes rollback note for user-facing or runtime-sensitive changes.
