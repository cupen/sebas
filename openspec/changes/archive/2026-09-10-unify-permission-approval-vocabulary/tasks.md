## 1. Spec alignment

- [ ] 1.1 Apply the `agent-driver` MODIFIED block (escalate native-only + ACP demote-to-allow_once SHALL).
- [ ] 1.2 Apply the `permission-flow` MODIFIED block (re-scope to Feishu render + hook allowlist; cede cross-driver vocab/namespace to agent-driver).
- [ ] 1.3 Apply the `acp-driver` MODIFIED block (rename hang "escalate" to "kill ladder"; note ACP children lack hang detection).

## 2. Code/comment reconciliation (ratify, no behavior change)

- [ ] 2.1 Confirm `session_backend.rs` maps `Escalate{reason}` → ACP `AllowOnce` and logs the downgrade; add a test asserting the demotion if absent.
- [ ] 2.2 Sweep comments for stale "escalate" kill-ladder wording; align to "kill ladder".

## 3. Glossary

- [ ] 3.1 Add the escalate 二义消解 and ACP 二义消解 entries (done in reconcile-glossary-with-im-service; keep in sync).

## 4. Validation

- [ ] 4.1 `openspec validate --changes` passes; archive checklist run.
