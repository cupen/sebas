## 1. Scaffold & Proposal

- [x] 1.1 Run `openspec new change bootstrap-specs` and confirm `.openspec.yaml` is created
- [x] 1.2 Write `proposal.md` (Chinese, <500 words, includes Non-goals section) and confirm it lists exactly two new capabilities: `permission-flow` and `acp-driver`

## 2. Pilot Specs

- [x] 2.1 Write `specs/permission-flow/spec.md` (English, with `## Purpose` ≥50 chars, `## ADDED Requirements`, every requirement has ≥1 `#### Scenario` with WHEN/THEN) covering: hook-driven request, three decision outcomes, auto-approve on allowlist hit, allowlist scope/lifetime, stale-click handling, fail-closed on missing responder
- [x] 2.2 Write `specs/acp-driver/spec.md` (English, same format rules) covering: one subprocess per session, startup handshake + timeout + hermetic env, resume rejection fallback, streaming event pump, cancel via interrupt+respawn, hang detection with escalating kill, single terminal event guarantee, provider env injection, permission reply routing

## 3. Design & Tasks

- [x] 3.1 Write `design.md` with Context / Goals+Non-Goals / Decisions (D1–D6) / Risks / Migration / Open Questions; confirm no open question would change the specs
- [x] 3.2 Write `tasks.md` using `- [ ] X.Y` checkbox format with verification in each task

## 4. Validation & Archive

- [x] 4.1 Run `openspec validate bootstrap-specs --strict` and confirm it exits 0
- [x] 4.2 Run `openspec status --change bootstrap-specs --json` and confirm all four artifacts (proposal, specs, design, tasks) report `done`
- [x] 4.3 Hand off to user for review; on approval run `openspec archive bootstrap-specs` and confirm `openspec/specs/permission-flow/spec.md` and `openspec/specs/acp-driver/spec.md` exist with `## Purpose` preserved

## 5. Next-Batch Setup (post-archive)

- [x] 5.1 Create a beads issue listing the remaining 15 capability specs to backfill — created `sebas-61l` (backfill remaining 15 capability specs)
