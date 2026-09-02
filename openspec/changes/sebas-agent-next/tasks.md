## 1. Policy engine & sandbox scaffold (design N1/N2, agent-core spec)

- [x] 1.1 Add the `policy/` module — `PolicyEngine`, `PolicyConfig` (static
      allow/deny rules), `PolicyDecision::Allow/Deny/Ask/Escalate`, and the
      three-layer evaluation (static rules → session allowlist → interactive
      ask, deny > allow > ask, no ask after deny); verify a unit test covers
      layer priority and fail-closed with no answerer
- [x] 1.2 Add the session allowlist (`(tool, args)` exact signature,
      consulted before any ask; allow-once vs allow-session semantics) and
      wire it into the engine; verify a test asserts an allowlisted exact
      signature runs silently and a different args variant still prompts
- [x] 1.3 Add `PermissionRequest` emission + ID correlation (request_id ==
      tool_use_id) and the `permission_decision` outcome path; hook the
      decision into the tool executor via a one-shot answer channel in
      `ToolCtx`; verify a test walks a gated call → request → allow-once →
      run-once → no allowlist, and a no-answerer case resolves as denied
- [x] 1.4 Add the Landlock sandbox backend (default) — the `landlock` crate
      enforced inside the bash child via `pre_exec`: handle fs from_all
      (BestEffort) plus net BindTcp/ConnectTcp (HardRequirement), read-only `/`,
      writable workdir + /tmp (directory rights) and /dev/null + /dev/urandom
      (file-level rights), zero AccessNet rules = deny-all TCP bind/connect;
      enforcement check on LandlockStatus + no_new_privs with the RulesetStatus
      honestly reported in `[bash conf: <mode>]`; any error → automatic
      fallback to firewall, never half-confined; verify with a test battery —
      write inside workspace succeeds, write to $HOME is denied, TCP connect is
      denied while an unrestricted control run gets connection-refused, and a
      simulated unsupported kernel takes the fallback path
- [x] 1.5 Add the firewall fallback backend — env scrubbing of known secret
      patterns (*_KEY/*_TOKEN/*_SECRET/*_PASSWORD, SEBAS_*), command -v/readlink
      literal probes refusing exact dangerous binaries, and the honest
      `[bash conf: firewall]` note in tool output, plus the
      `is_likely_sandbox_denied` heuristic (exit codes 2/126/127 + output
      keywords) as an annotation only — never altering the structured decision;
      verify a test asserts the firewall refuses a dangerous absolute binary
      and reports its profile honestly

## 2. Web tool surface (design N3, agent-core spec)

- [x] 2.1 Add `web_fetch` — URL scheme allowlist (http/https only), bounded
      redirect hops (≤3), output cap (~100KB) with truncation marking, ~30s
      timeout, best-effort robots.txt; verify tests cover scheme rejection,
      cap truncation, and timeout
- [x] 2.2 Add `web_search` — query → capped result entries (≤8) with
      truncation marking and no redirect chasing; verify cap and no-network
      tests
- [x] 2.3 Register both under the network policy gate — default `off` (tool
      returns a structured "network disabled" result, no network request),
      `ask` (goes through the review card), `allow` (silent); verify a test
      asserts the disabled case makes no network call and the ask case emits a
      `PermissionRequest`

## 3. Context management & concurrency (design N4, agent-core spec)

- [x] 3.1 Implement tool-result rewriting — the stored transcript holds a
      first-~8k placeholder with a `[truncated]` marker, deterministic,
      described in tool descriptions; the `ToolEnd` event keeps the pre-rewrite
      capped version and the model-visible tool_result always carries a marker
      with no silent drops; verify tests for >cap output, marker presence, and
      description mention
- [x] 3.2 Add the Assembly budget — `max_messages` (default 80) and an
      estimated-token gate ahead of each model call; exceeding either finishes
      the turn cleanly as budget-exhausted (never an error), session reusable;
      verify one test per limit
- [x] 3.3 Add concurrent read-only tool execution — read/glob/grep/web_*
      parallel under `max_concurrent_readonly` (default 8), writes serialize
      after the read-only group, ToolStart in response order and tool_result
      appended in response order regardless of completion order; verify a
      mixed-group test asserts ordering and a concurrent read-only test asserts
      the events stay in response order

## 4. Event vocabulary & tools upgrade (design N6, agent-core spec)

- [x] 4.1 Add `ToolFinish`, `SessionSummary`, and enable `PermissionRequest` on
      `AgentEvent`; add the `ContentBlock::Image` variant and multi-modal
      request support; verify serde round-trips and the existing 1a
      event-sequence tests still pass unchanged
- [x] 4.2 Add `read_image` behind a capability gate — declared only when the
      configured LLM announces image support; and a `lsp` tool that reports
      `file_system` only when a server is reachable and returns unavailable
      (not error) otherwise; verify both gates with a fake client that does and
      does not announce the capability
- [x] 4.3 Register the new tools conditionally in `ToolRegistry` and expose the
      `LlmConsult` constants (tools ≤128, context ~90% finish); verify the
      schema list reflects the capability gates

## 5. Webui wiring (design N5, webui spec)

> **谱系裁决（2026-09-02，用户确认）**：以 feat/webui 谱系为准——router 回
> SessionInfo/TurnEntry/Resync 词表 + transcript 机制，webui 取 SPA+rust-embed 全套，
> binary 改接 session_backend 缝（backend.rs/webui_backend.rs 退役）。5.2/5.3 的
> API 与 /ws 层已就绪（spawn backend 提示、/api/permissions answer、
> permission.requested WS 帧）；SPA 组件（会话行下拉、审查卡）留待前端轮次。

- [x] 5.1 Add the `NativeAgentBackend` adapter in the sebas binary crate over
      `SessionManager` — create/prompt/cancel/close/events mapped onto the
      `SessionBackend` trait; verify it satisfies the trait and drives a fake
      client through `run --webui`
- [x] 5.2 Add the session-row backend selector (acp / native drop-down) and
      route `spawn`/`message`/`close`/`turns` to the chosen backend; verify the
      acp path behaves exactly as before and the native path renders a live
      session
- [x] 5.3 Add the review card — webui renders `PermissionRequest` (allow once /
      allow session / deny + reason) and returns `permission_decision` to the
      session; verify a gated call via the native backend is answered through
      the card end-to-end
- [x] 5.4 Verify `cargo build -p sebas-webui` and `cargo test -p sebas-webui`
      pass with no regression in the acp-mode tests, and `run --webui` still
      drives an acp session as before

## 6. agent-bench (agent-bench spec)

- [x] 6.1 Add `sebas agent-bench` — `--smoke`, `--tasks a,b,c`, `--model m`,
      `--record trace.jsonl`, `--debug`, `--replay`; run a task in a temp
      workspace, record the full event stream to JSONL with a `# task:` header,
      and print a per-task summary; verify the CLI runs the smoke subset against
      a fake client and the trace file exists
- [x] 6.2 Add per-task assertions on final workspace state and trace content
      (fail-fast on missing files, no prose inspection); add the
      ERROR-RECOVERY task (fixture command fails early, score depends on
      recovery); verify a passing and a failing task each report correctly
- [x] 6.3 Add the tree dashboard — fixed task order, buckets (web-tooling /
      apply_patch / subagent, the latter two placeholder-marked skipped in this
      change), per-bucket roll-up, deterministic ordering; verify the printed
      tree matches the expected layout for a smoke run
- [x] 6.4 Add honest environment reporting — client type, model, tool list,
      budgets, runtime, sample count in the trace header and summary; verify a
      run prints these and the trace header carries them

## 7. Blueprint update & final gate

- [x] 7.1 Revise design doc §3 (Codex teardown) — upgrade evidence to source
      (`openai/codex`, 2026 snapshot): CX-1 corrected (Linux sandbox default is
      bubblewrap + seccomp, Landlock is the legacy fallback; macOS sandbox-exec;
      Windows RestrictedToken), CX-3 corrected (the single-tool-surface verdict
      is superseded — unified_exec / apply_patch / update_plan + MCP +
      multi-agent + code_mode), plus new adopt rows (session approval cache
      ApprovedForSession, is_likely_sandbox_denied, rollout-trace, codex exec
      headless events); verify every revised row cites a source and no uncited
      load-bearing claim remains
- [x] 7.2 Revise design doc §4 (DSH teardown) — upgrade `S9 [观测]` to
      `S10 [源码]` with deepseek-ai/deepseek-harness (2026-08-13, MIT) and
      official docs citations; correct mechanism rows (exit_plan_mode,
      workflow, lsp, session_search, jump policy); verify every revised row
      cites a source and no uncited load-bearing claim remains
- [x] 7.3 Revise design doc §11 (roadmap) — Phase 2 = permission/sandbox +
      network surface (this change); Phase 3 splits into 3a compaction, 3b
      todo + proactive ask, 3c plan mode + apply_patch; Phase 4 subagent/MCP/
      skills registry; persistence (OQ1) promoted to an explicit roadmap item
      (Codex thread-store/SQLite and DSH session-log both argue it); add §12
      revision ledger; verify the roadmap and the delivered change scope agree
- [x] 7.4 Full gate — `cargo build` workspace-wide, `cargo test -p sebas-agent`
      and `-p sebas-webui`, `cargo clippy -p sebas-agent -p sebas-webui -- -D
      warnings`; verify all pass, existing crates untouched (`git status` shows
      only this change's files), and `openspec validate sebas-agent-next`
      passes