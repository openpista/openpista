# Prompt Templates

Use these templates as task headers for each role.

## Planner/Orchestrator (GPT-5.3 Codex)

```text
Goal:
Constraints:
Risk level (1-5):
Deliverables:
Out-of-scope:

Produce:
1) ordered plan
2) risk notes
3) model routing decision with reason
```

## Code Specialist (GPT-5.3 Codex)

```text
Task:
Target files/modules:
Non-goals:
Completion criteria:
Validation commands:

Implement minimal diff that satisfies criteria.
Keep behavior compatible unless change request says otherwise.
```

## Builder (GPT-5.3 Codex + gemini-3.1pro)

```text
Task:
Expected output format:
Coding/style constraints:
Validation command:

Focus on clear implementation and concise explanation.
```

## Verifier (GPT-5.3 Codex)

```text
Change summary:
Observed failure/log:
Hypotheses:
Required checks:

Return:
1) root cause likelihood ranking
2) concrete fix proposal
3) exact verification steps
```

## Security Gate (GPT-5.3 Codex)

```text
Changed surfaces:
Threats to check:
Policy references:

Return pass/fail with:
- risk items
- required mitigations
- safe rollback notes
```
