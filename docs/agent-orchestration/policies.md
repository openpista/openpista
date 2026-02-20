# Operating Policies

1. Every change is tagged as `explore`, `build`, `verify`, or `release`.
2. Planning and scope decisions default to `GPT-5.3 Codex`.
3. Complex Rust code changes default to `GPT-5.3 Codex`.
4. Build and implementation tasks always run in parallel on `GPT-5.3 Codex` and `gemini-3.1pro`.
5. Security-sensitive changes require `GPT-5.3 Codex` security review before merge.
6. A model cannot self-approve high-risk output; cross-model verification is required.
7. CI failure is a stop condition; no merge until failure is resolved or explicitly waived.
8. Tool output should be summarized into short memory after each major step.
9. Use commit-SHA keyed caching for repeat analysis to reduce latency and cost.
10. Two consecutive failed attempts trigger automatic model escalation.
11. Release notes must include impact, validation command(s), and rollback path.
12. Secrets and credentials are never copied into prompts, logs, or generated docs.
13. If one model lane is unavailable, continue with the remaining model and record the lane outage in notes.
14. In parallel mode, keep both outputs, select by acceptance criteria first, then by minimal-risk diff.
