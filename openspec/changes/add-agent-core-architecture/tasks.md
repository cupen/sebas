## 1. Scaffold & sources

- [x] 1.1 Create `docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md`
      with the two-part skeleton (Part I research teardowns, Part II target
      architecture) and the fixed mechanism-table template from design D5; verify
      the file exists with all planned headings and an empty table in each
      teardown section
- [x] 1.2 Collect primary sources for the three reference agents (public docs,
      changelogs, release notes, observable CLI behavior; mark anything inferred)
      into the document's source list; verify each teardown section has at least
      two cited sources and no uncited load-bearing claim

## 2. Research teardowns (Part I)

- [x] 2.1 Write the Claude Code teardown — agent loop shape, toolset and per-tool
      contracts, permission prompting, hooks/sub-agents, streaming card-style
      updates — as a mechanism table with a transfer verdict per row; verify every
      row ends in adopt / adapt / skip with a one-line why grounded in sebas
- [x] 2.2 Write the Codex teardown — sandbox and approval modes, loop and tool
      surface, what its confinement model implies for a Rust tool executor; verify
      the same per-row verdict format and that unverified internals are labeled as
      inference
- [x] 2.3 Write the DeepSeek-Harness teardown — file-sandbox policy and escalation,
      approval flow, goal/job orchestration, skill system, sub-agents; verify the
      same format and that each mechanism names where it would attach in sebas
- [x] 2.4 Write the cross-agent synthesis — shared invariants vs deliberate
      divergences, and an operational definition of "professional coding agent" as
      a checklist; verify every checklist item traces back to at least one teardown
      row

## 3. Target architecture (Part II)

- [x] 3.1 Design the in-process agent loop — states, cancellation safety, event
      streaming (text / thinking / tool call / tool result), stop conditions and
      budgets — with each decision naming its crate seam; verify the loop consumes
      `gateway` via the Anthropic Messages protocol and streams events compatible
      with the existing card/SSE model
- [x] 3.2 Define the uniform tool interface (one trait, JSON-Schema parameters,
      structured tool results) and per-tool contracts for bash / read / write /
      edit / glob / grep; verify each tool has parameter schema, result and error
      semantics, and an explicit non-goal where scope is limited
- [x] 3.3 Sketch the session integration — a `SessionBackend`-shaped API (create,
      prompt, cancel, SSE event stream) that webui consumes first and Feishu/CLI
      attach to later; verify the sketch matches the `add-core-session-channel`
      seam and names no new coupling into router or Feishu behavior
- [x] 3.4 Write the evolution roadmap — core loop → permission/sandbox → context
      management → sub-agents/MCP — with entry criteria per phase; verify later
      phases stay at roadmap depth per the proposal's non-goals

## 4. Review & cross-check

- [x] 4.1 Cross-check every code reference in the document against the current
      tree (acp-claude driver/manager, gateway protocols, webui `SessionBackend`
      seam, SSE path); verify each cited file path and symbol resolves by search
- [x] 4.2 Consistency pass across proposal, design, and the document — verdicts
      match decisions D1–D7, roadmap matches non-goals — then run `openspec
      validate` for this change; verify validation passes and no artifact is
      inconsistent with the delivered document
