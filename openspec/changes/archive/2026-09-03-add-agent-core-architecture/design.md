## Context

See `proposal.md` — Why. Current state that shapes this change:

- **sebas drives, but is not, an agent.** `acp-claude` spawns an external `claude` CLI
  subprocess per session (SessionManager + CcDriver). sebas owns transport, routing,
  cards, and the gateway — but the agent loop, tool contracts, and context management
  live inside Claude Code's process. There is no in-process coding agent anywhere in
  the workspace.
- **The LLM seam already exists.** `gateway` terminates both Anthropic and OpenAI
  wire protocols and routes by model name to any upstream provider, with usage
  recording. A native agent can speak the Anthropic Messages protocol to `gateway`
  exactly like Claude Code does today.
- **The webui groundwork exists.** `redesign-webui-console` (token system, vendored
  assets, SSE consumption, status vocabulary) and `add-core-session-channel`
  (`SessionBackend` seam, detached vs in-process core) are the substrate the workbench
  builds on. A native agent's events can ride the same SSE path.
- **This change ships documents only.** The deliverable is one research + architecture
  design document under `docs/superpowers/specs/`, following the existing design-doc
  convention. Future implementation changes will derive their specs and tasks from it.

## Goals / Non-Goals

**Goals:**

- Answer "what makes a professional coding agent" with mechanism-level teardowns of
  Claude Code, Codex, and DeepSeek-Harness (agent loop, tool system, context
  management, permission model, streaming interaction) — each ending in a
  transferable-pattern verdict for sebas, not a generic survey.
- Define the target architecture for a sebas-native agent core: an in-process agent
  loop over a basic toolset (bash / read / write / edit / glob / grep), LLM access via
  `gateway`, webui-first interaction — with module boundaries against `acp-claude`
  and an evolution roadmap (core loop → permission/sandbox → context management →
  sub-agents / MCP).
- Make the document actionable: every design decision names the existing crate it
  plugs into, so the first implementation change can be scoped directly from it.

**Non-Goals:**

- No code, crates, dependencies, or spec deltas (`skip_specs: true`).
- Full designs for permission/sandbox, context compaction, sub-agents, or skills/MCP —
  the roadmap names them as phases; only the core loop + basic toolset is designed.
- Model benchmarking or cost comparison.
- Reproducing proprietary internals: teardowns rely on observable behavior and public
  sources, with inference explicitly marked as such.

## Decisions

- **D1 — In-process event-driven agent loop, not another subprocess bridge.** The
  core owns the loop: LLM request → tool calls → tool results → repeat until no tool
  call or budget exhausted. Alternative considered: keep bridging external CLIs
  (status quo) — rejected because the stated goal is to own the agent; and wrapping
  an existing agent SDK — rejected because tool contracts and context policy are
  exactly what we want to control. The loop must be cancellation-safe and stream
  every event (text, thinking, tool call, tool result) as it happens, matching the
  card-streaming model sebas already has.
- **D2 — Uniform tool interface: one trait, JSON-Schema-declared parameters, first
  toolset = bash / read / write / edit / glob / grep.** Tool results return as
  structured tool-result messages, mirroring Claude Code's toolset shape. Alternative:
  a single do-everything bash tool — rejected; per-tool contracts give the model
  better affordances, give permission checks a surface to attach to later, and keep
  transcripts legible (a `edit` call says more than an opaque `sed` in bash output).
- **D3 — LLM channel = `gateway`, Anthropic Messages protocol first.** agent-core
  depends on the protocol, never on a provider. Alternatives: direct provider SDKs —
  rejected (duplicates routing/usage already in gateway); OpenAI protocol first —
  rejected (Anthropic tool-use semantics are the shape Claude Code proves out, and
  gateway speaks both anyway).
- **D4 — Webui-first via a session/turn/event API, not a bespoke UI path.** agent-core
  exposes an in-process session surface shaped like `add-core-session-channel`'s
  `SessionBackend` seam: create session, send prompt, cancel, consume SSE events.
  Feishu and CLI later attach to the same API. Alternative: build straight into webui
  routes — rejected; it would couple the core to one surface and repeat the RouterHandle
  coupling that `add-core-session-channel` is unwinding.
- **D5 — Teardown method: mechanism tables + transfer verdicts.** For each reference
  agent (Claude Code: toolset, permission prompts, hooks, sub-agents; Codex: sandbox
  and approval modes; DeepSeek-Harness: file sandbox, approval policy, goal/job
  orchestration, skill system), a fixed table — mechanism, how it works, evidence,
  transfer to sebas (adopt / adapt / skip, why) — plus a cross-agent synthesis section.
  Alternatives per row are recorded, so the document doubles as decision history.
- **D6 — One document, ADR-style decisions.**
  `docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md`, research
  front / design back, each design decision in adopt-alternatives-rationale form.
  Alternative: separate research and design documents — rejected; the design cites
  research verdicts constantly, and one file cannot drift apart from itself.
- **D7 — Coexist with `acp-claude`; replace nothing.** agent-core is a second
  execution backend selectable by the session layer; `acp-claude`, router command
  handling, and Feishu behavior are untouched. Rollback is trivial: stop selecting the
  backend (and for this change, delete one markdown file).

## Risks / Trade-offs

- [Teardowns degrade into a shallow survey] → The fixed mechanism-table format and a
  per-agent acceptance clause in tasks.md force verdicts; any section without a
  transfer verdict fails review.
- [Design floats free of the codebase] → Every decision must name the crate/seam it
  plugs into (`gateway`, `SessionBackend`, SSE path); tasks include a cross-check pass
  against current code, citing file paths.
- [Scope creep toward an implementation manual] → Non-goals pin permission/sandbox,
  compaction, sub-agents, MCP to roadmap mentions; reviewers reject detailed designs
  of them.
- [Public information on Codex / DeepSeek-Harness internals is partial] → Teardowns
  state observable behavior + cited sources; inference is labeled, and the design
  never leans on an unverified internal detail for a load-bearing decision.

## Open Questions

- Whether agent-core session persistence reuses `session-lifecycle`'s state-file
  machinery or gets its own store — implementation-change territory; the document
  records the options, not the choice.
- Whether MCP enters the tool interface from day one or lands as a later adapter —
  the roadmap mentions both; the basic toolset design assumes only the native trait.
