# Fallback Runbook

Use this runbook when the default execution lane fails.

## Trigger Conditions

- Same task fails twice on same model.
- Build/test errors are non-deterministic across retries.
- Proposed fix passes local checks but fails CI gate.
- Security-sensitive area has unresolved review comments.
- One parallel model lane is unavailable.

## Response Steps

1. Freeze scope and capture current hypothesis in 5 lines or less.
2. Re-run failing command once with full logs attached.
3. Planning remains on `GPT-5.3 Codex`.
4. Build/implementation remains parallel on `GPT-5.3 Codex` + `gemini-3.1pro`.
5. If one parallel lane is down, continue with the available lane and log reduced redundancy.
6. If still unresolved, route to `Planner` (`GPT-5.3 Codex`) for plan rewrite.
7. Apply smallest safe patch and re-run required checks.
8. If issue is production-facing, include rollback command in PR notes.

## Incident Notes Template

```text
Task:
Failure signature:
What changed:
Current best hypothesis:
Next action:
Owner:
```

## Exit Criteria

- All required checks pass.
- Root cause is documented.
- Follow-up action item is created if workaround was used.
