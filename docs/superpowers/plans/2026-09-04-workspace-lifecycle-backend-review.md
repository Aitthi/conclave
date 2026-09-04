# Review workspace and agent lifecycle backend

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Artifact

Review commit stack `c1613f9725b0`, `934713c3b9e9`, `5c232a80e0f0` on branch `lane/workspace-agent-lifecycle-backend` against task `workspace-agent-lifecycle-backend` and rulings `2d72d06e`, `87af7d7f`, `3c8d66a1`.

## Review method

Use an outsider, end-to-end review. First state whether the two-level persisted state model is the smallest correct solution. Then trace real paths, not only diff hunks:

- v26 -> v27 and fresh DB migration; existing user workspace stopped; hidden/internal draft explicitly started; relations preserved.
- UI/CLI router entry -> workspace start/stop -> lock -> DB linearization -> per-agent launch/teardown -> runtime/browser/event output.
- Agent stop/resume, failure rollback and idempotency; generic spawn/restart bypass attempts.
- Workspace/agent eligibility for message send/inject and task claim, including queued-row prevention and watcher best-effort behavior.
- Stop racing Resume/spawn/message/restart tails/late EOF, lock ordering, delete/remove/agent-def delete behavior.
- Browser ended/resumed state and supplemental wrapper.
- CLI syntax/help/response serialization and test coverage.
- Diff-scoped formatting exception and whether any feature hunk remains unformatted.

## Deliverable

Read-only review note ordered by blocker/major/minor, each with file:line evidence, consequence, and minimal fix. Explicitly call out any plan requirement that is unimplemented or only superficially tested. Finish with SHIP or FIX-THEN-SHIP. Do not edit product files.
