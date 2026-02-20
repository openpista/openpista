# Multi-Agent Architecture

This document describes the end-to-end multi-agent flow used in this repository.

## End-to-End Flow (ASCII)

```text
                         +----------------------+
User Request ----------->| Planner/Orchestrator |  (GPT-5.3 Codex)
                         +----------+-----------+
                                    |
                                    v
                         +----------+-----------+
                         | Parallel Lane Check  |
                         +----+-------------+---+
                              |             |
                        both up           one down
                              |             |
                              |             v
                              |   +--------------------------+
                               |   | Build/Impl Parallel Mode |
                               |   | Codex + gemini-3.1pro    |
                              |   +-------------+------------+
                              |                 |
                              +-----------------+
                                    |
                                    | 1) task decomposition
                                    | 2) risk scoring
                                    | 3) model routing
                                    v
                    +---------------+----------------+
                    | Task Router (score/risk based) |
                    +--------+------------------------+
                             |
        +--------------------+---------------------+
        |                                          |
        v                                          v
+------------------------+              +------------------------+
| Builder Lane           |              | Code Specialist Lane   |
| Codex + gemini-3.1pro  |              | Codex + gemini-3.1pro  |
| - low/medium risk work |              | - complex code changes |
| - docs/polish updates  |              | - Rust logic/debugging |
+-----------+------------+              +------------+-----------+
            \                                        /
             \                                      /
              +----------------+-------------------+
                               |
                               v
                   +-----------+------------+
| Verifier (Cross-check) |
| GPT-5.3 Codex          |
                   | - tests/regression      |
                   | - root-cause validation |
                   +-----------+------------+
                               |
                               v
              +----------------+-------------------+
| Security Gate (conditional required)|
| GPT-5.3 Codex                      |
              | - system.run/auth/session/CI perms |
              +----------------+-------------------+
                               |
                     pass      |      fail
                               |
                 +-------------+-------------+
                 |                           |
                 v                           v
        +--------+---------+        +--------+---------+
        | Merge / Release  |        | Fallback Router  |
        | - PR/release note|        | - retry/escalate |
        +------------------+        | - reroute model  |
                                    | - replan on Codex|
                                    +--------+---------+
                                             |
                                             +-----> back to Planner
```

## Routing Summary

- Low score tasks: route to `Builder` (parallel `GPT-5.3 Codex` + `gemini-3.1pro`).
- Medium/high complexity tasks: route to `Code Specialist` (`GPT-5.3 Codex`).
- Security-sensitive changes: require `Security Gate` (`GPT-5.3 Codex`) before merge.
- Repeated failures: trigger fallback routing and escalation.
- Build/implementation always run in parallel on `GPT-5.3 Codex` and `gemini-3.1pro`.

## Why This Works

- Separates planning from implementation to reduce routing mistakes.
- Uses fixed model policy: plan on `Codex`, implementation on `Codex + gemini-3.1pro`.
- Keeps a hard safety boundary with dedicated `Security Gate` checks.
- Maintains progress under lane outages by degrading to single-lane execution with notes.
