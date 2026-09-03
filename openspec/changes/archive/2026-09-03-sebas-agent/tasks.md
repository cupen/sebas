## 1. Crate scaffold & core types

- [x] 1.1 Create the `sebas-agent` workspace member with modules `llm/` `loop/`
      `tools/` `session/` and the dependency set from design N3 (tokio
      process/fs, reqwest stream/json, eventsource-stream, async-trait, regex,
      globset, walkdir, futures-util); verify `cargo build -p sebas-agent`
      passes and the crate appears in the workspace member list
- [x] 1.2 Define the message/tool data model: `Message`, content blocks
      (text / thinking / tool_use / tool_result), `ToolResult`, `ToolErrorKind`,
      and turn-budget config struct; verify unit tests construct and
      serialize a full turn transcript round-trip

## 2. LLM client (design N5)

- [x] 2.1 Implement the `LlmClient` trait and `FakeLlmClient` in both scripted
      (preset response sequence) and stateful (closure over last tool result)
      modes; verify a fake drives a two-round conversation in a unit test
- [x] 2.2 Implement `AnthropicMessagesClient`: Anthropic Messages streaming
      request against a configured endpoint — direct provider base URL + API
      key (default), or gateway URL + auth token — with the SSE event table
      from design N5; verify against a recorded frame fixture covering
      `input_json_delta` fragmentation and a chunk boundary split (spec:
      tool-arguments scenario)
- [x] 2.3 Map stream events to the crate event type with `TextDelta` /
      `ThinkingDelta` emitted on arrival and tool calls queued only at
      `content_block_stop`; verify the fixture test asserts ordering and no
      premature tool execution

## 3. Tool executor (design N6)

- [x] 3.1 Implement the `Tool` trait with `ToolCtx` (workdir, cancellation
      token, progress sink, read-tracking set) and register the six tools;
      verify `cargo test -p sebas-agent tools` lists all six with valid
      JSON schemas
- [x] 3.2 Implement bash with process-group spawn, killpg on timeout and
      cancel, tail-capped output (~30k), and nonzero exit surfaced as
      `ok:true` carrying the exit code; verify a test spawns a sleeping child,
      cancels, and asserts no orphan survives (spec: timeout / cancel scenarios)
- [x] 3.3 Implement read (line-numbered, offset/limit paging, binary/dir
      detection), write and edit (read-before-write refusal, exact-match
      edit reporting match count, tmp+rename atomic write); verify tests cover
      each refusal and the edit match-count error (spec: write/edit scenarios)
- [x] 3.4 Implement glob (walkdir + globset, mtime order, 100-cap with
      truncation flag, skip `.git`/`target`) and grep (regex, include filter,
      per-file grouping, 250-cap); verify tests with >100 and >250 matches
      assert the caps and truncation flags (spec: glob/grep scenarios)

## 4. Turn loop engine (design N7/N8)

- [x] 4.1 Implement the turn state machine (Idle → AwaitingModel ⇄
      ExecutingTools → Finished/Cancelled/Failed) with the three-leg
      `tokio::select!` (command / step / deadline); verify a scripted test
      drives a five-plus-tool multi-step turn to a clean finish (spec:
      multi-step scenario; C1)
- [x] 4.2 Implement the three budgets (model calls, tool calls, wall-clock)
      ending in a terminal finished event marked budget-exhausted — never an
      error; verify one test per limit (spec: budgets scenario; C8)
- [x] 4.3 Implement cancellation: token propagation into `ToolCtx`, process
      termination, history preservation, session reusable for the next prompt;
      verify cancel mid-bash kills the child, emits the cancellation outcome,
      and a follow-up prompt runs normally (spec: cancellation scenarios; C7)

## 5. Session manager & prompt assembly (design N4/N7)

- [x] 5.1 Implement `SessionManager` / `SessionHandle` with per-session task,
      mpsc commands, broadcast `AcpEvent` stream, and serial prompt queueing;
      verify two concurrent sessions never observe each other's events or
      workdir (spec: session lifecycle scenarios)
- [x] 5.2 Implement system-prompt assembly: sebas-agent identity section +
      workdir note + AGENTS.md then CLAUDE.md injection when present; verify a
      test asserts the system content with memory files present and with both
      absent (spec: memory scenario; C6)
- [x] 5.3 Map crate-internal events onto the existing `AcpEvent` vocabulary
      with zero new variants and no permission-request emission; verify an
      integration test asserts the event sequence over a full turn
      (spec: streaming vocabulary; C2)

## 6. Example & integration scenarios

- [x] 6.1 Add `examples/agent-dev.rs`: headless host reading prompt and
      workdir from args, provider endpoint (direct by default) or gateway
      from env, printing events to stderr; verify `cargo run --example
      agent-dev -- --help` runs without touching the CLI command table
- [x] 6.2 Integration tests in `sebas-agent/tests/`: scripted
      five-plus-tool loop, stateful self-heal (bash exits nonzero, model
      recovers), cancel mid-bash, budget exhaustion, two-session isolation;
      verify `cargo test -p sebas-agent` is green and each spec scenario has a
      named test

## 7. Final verification

- [x] 7.1 Run the full gate: `cargo build` workspace-wide, `cargo test -p
      sebas-agent`, `cargo clippy -p sebas-agent -- -D warnings`; verify all
      pass and existing crates are untouched (`git status` shows only the new
      crate, example, and this change's artifacts)
- [x] 7.2 Consistency pass: spec scenarios ↔ test names, design N1–N10 ↔ code
      layout, `openspec validate sebas-agent` passes; verify no artifact is
      stale before hand-off
