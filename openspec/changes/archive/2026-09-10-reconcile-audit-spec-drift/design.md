## Context

The two-sweep audit found spec drift of three kinds: (a) code bugs (fixed separately in beads), (b) intentional code evolution the specs never caught — security hardening, extract-im-service, sebas-ixv interaction changes, (c) spec over-promises for unbuilt features. This change records the batch (b) spec-text corrections plus the two smallest (c)/(a) items fixed alongside.

## Goals / Non-Goals

**Goals:**
- Make every corrected spec state the deployed behavior truthfully, with the rationale (which commit/change made code the intended behavior) recorded in this change's proposal.

**Non-Goals:**
- No behavior changes beyond the two bundled small fixes (agent-bench CLI, up-to-date no-restart).
- Does not touch specs owned by the operator's in-flight capability re-split.

## Decisions

- Fix main specs in place (skip_specs change for accounting) — same pattern as repair-core-session-channel-spec. Deltas were unnecessary because every edit restates current behavior, not new behavior.
- up-to-date no-restart fixed in code (spec was right): restarting core on a no-op update kills live sessions for nothing; EXIT_UP_TO_DATE=3 distinguishes the no-op outcome through the existing subprocess boundary.

## Risks / Trade-offs

- Reader expecting `--log-file` per the old fail-fast proposal text: the design contract (D2) only ever committed SEBAS_STARTUP_ERROR_FILE; the flag was never implemented.
- External dashboards built on the old `sebas_router_*` series names (fixed in sebas-cgq) must migrate to `router_*` — that rename was spec-mandated from the start.
