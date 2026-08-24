## Context

See `proposal.md` — Why. The two pilot capabilities sit on opposite sides of the router ↔ agent boundary:

- `permission-flow` is inherently **cross-cutting**: it starts inside `acp-claude` (PreToolUse hook), crosses into `router` (card dispatch, allowlist, button callback), and ends in `feishu` (card rendering + flip). A spec that pretends it lives in one crate would lie.
- `acp-driver` is **single-crate** (`acp-claude`) but has heavy external surface: the Claude CLI's argument quirks (`--session-id` vs `--resume`), the SDK's initialize handshake, and its stream-json + control protocol.

Historical context that shapes what's in scope: the project used to route permissions through an out-of-process ACP bridge + unix-socket broker (see `docs/superpowers/specs/2026-08-01-acp-bridge-*`). That design is **retired**; these specs describe only the current direct-SDK architecture (`docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md`).

## Goals / Non-Goals

**Goals:**

- Capture **observable** behavior (router outward, driver outward), not function names.
- Keep scenarios **testable** — each `#### Scenario` should be convertible into a test in `acp-claude/tests/` or `router/tests/` without inventing new behavior.
- Pin down the **invariants** listed in `docs/perm-flow/sequence.md` (one oneshot per request_id, fail-closed, allowlist scope) as first-class requirements.
- Leave **OpenSpec ID + requirement naming conventions** stable enough that the next 15 capabilities can be filled in mechanically.

**Non-Goals:**

- Do not spec the Feishu card layout, copy text, or button styling — that's `feishu-cards`'s job (later change).
- Do not spec how the router maps chat→session (`session-lifecycle`, later) or how state is persisted (`session-persistence`, later).
- Do not spec provider resolution logic (`provider-management`, later) — `acp-driver` only covers the `extra_env` injection seam it consumes.
- Do not document deprecated behaviors (ACP bridge, broker, hook script) even in a "Migration" section. They are gone.

## Decisions

### D1: Spec granularity = one file per capability, flat under `specs/`

Chosen: `specs/permission-flow/spec.md`, `specs/acp-driver/spec.md`. No nesting.

Alternatives considered:
- **Per-crate** (`specs/acp-claude/spec.md`, `specs/router/spec.md`): rejected — crates are implementation units; `permission-flow` already spans three of them.
- **Nested by layer** (`specs/agent/acp-driver/spec.md`): rejected — only two pilot files today; introducing a hierarchy now is premature.

### D2: `permission-flow` covers the full loop, including the hook callback contract

The spec talks about the **hook callback contract** ("the system SHALL park the hook callback until a decision is returned") because that's externally visible from the agent's perspective: Claude blocks on tool execution until the hook answers. Stopping the spec at "router emits event" would omit the actual guarantee the user cares about.

Alternatives considered:
- **Spec only the router side** (events in, button events out): rejected — the oneshot contract is the actual behavior; without it, "fail closed" is just a hope.
- **Push the hook contract into `acp-driver`**: rejected — the decision flow is one coherent behavior; splitting it across two specs would force readers to flip files to understand any single scenario.

### D3: Allowlist scope = `SessionKey` (chat + thread), not per user

This matches the implementation (`router/src/state.rs`). Alternatives:
- **Per user** (a user's `Allow session` carries across chats): rejected — would leak tool grants across trust boundaries (a group chat vs a private chat).
- **Per session id** (dies with `/new`): too narrow — would re-prompt on every `/new`, which users explicitly complained about (bead sebas-8ig).

### D4: Requirement granularity

Each `### Requirement` is a behavior cluster (e.g. "Three decision outcomes"), each `#### Scenario` is a single observable branch. We deliberately did NOT make every button click its own requirement — that would explode the file and bury the invariants.

### D5: Invariants stay in scenarios, not in a separate "Invariants" section

The "Key invariants" list from `docs/perm-flow/sequence.md` is folded into explicit requirements (`Fail-closed on missing responder`, `Stale click handling`, `Allowlist scope and lifetime`). A separate "Invariants" section would be non-normative and easy to skip.

### D6: Language

Spec body: **English**. Rationale: OpenSpec convention + tooling (search, linters, examples) assumes English. Project docs (proposal, design, README) stay in Chinese. This is a one-line decision recorded here so future changes don't re-litigate it.

## Risks / Trade-offs

- [Spec drifts from code as features land] → Mitigation: every future change that touches these behaviors MUST route through a delta spec in its own `changes/<name>/specs/`, and archive applies it. This is the OpenSpec contract; if we bypass it, the baseline rots.
- [Two pilot files don't cover cross-cutting behaviors like card streaming or session resume] → Accepted: those get their own capabilities in the next batch. Trying to squeeze them into `permission-flow` or `acp-driver` would muddy both.
- [English specs vs Chinese-speaking contributors] → Mitigation: `proposal.md` and `design.md` stay Chinese; specs use short, scenario-style English which is closer to test code than prose.
- [Tool names (`Allow once`, `Allow session`, `Deny`) are English in spec but Chinese in product ("仅本次" / "本会话")] → Accepted: spec uses the English token; rendering is `feishu-cards`'s concern.

## Migration Plan

None. This is a docs-only change. Archive flow:

1. `openspec validate bootstrap-specs --strict` — passes.
2. `openspec archive bootstrap-specs` — copies the two `spec.md` files into `openspec/specs/<capability>/spec.md` and moves this change under `openspec/changes/archive/`.
3. Subsequent capabilities are filled in via new changes (one per batch of 3–5).

Rollback: delete `openspec/specs/permission-flow/` and `openspec/specs/acp-driver/`, restore the change from `openspec/changes/archive/bootstrap-specs/`.

## Open Questions

None. The two capabilities chosen were specifically the ones with the most existing documentation (`docs/perm-flow/sequence.md`, the design spec at `docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md`), so no deferrable unknowns remain.
