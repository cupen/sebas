## Context

`openspec/glossary.md` is the single source of truth for terminology, but it predates `extract-im-service` and two later shifts. It self-contradicts (core "承载通道适配器(飞书 WS 等)" at line 11 vs "core 不注册任何 IM 适配器" at line 94), keeps the stale "出站呈现编排" wording for dispatch, and leaves "escalate" and "ACP" ambiguous (both have a deprecated meaning and a live meaning that history reads as contradictory).

## Goals / Non-Goals

**Goals:**
- Remove the internal contradiction about core hosting IM adapters.
- Update the dispatch definition to reflect IM-presentation ownership moving to sebas-im.
- Disambiguate "escalate" (approval vs kill ladder) and "ACP" (dead private bridge vs live standard).

**Non-Goals:**
- No spec/behavior change (this is a docs-only reconciliation; skip_specs).
- No rewrite of healthy glossary sections.
- No edits to `docs/design-history.md` ADRs.

## Decisions

- Edit the three glossary locations in place; add the two disambiguation entries under the existing 三义消解 block (precedent: router 三义).
- Keep the change scoped to terminology; link to the related permission/reactions changes rather than duplicating their content.

## Risks / Trade-offs

- Minimal. The only risk is terminology churn if a definition is still contested; the escalate/ACP entries ratify current code/spec reality.
