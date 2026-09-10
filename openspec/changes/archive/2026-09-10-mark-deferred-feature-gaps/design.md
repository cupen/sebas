## Context

Two-sweep audit + two repair rounds left exactly one class of drift: specs promising unbuilt features. The project precedent (align-session-map-persistence-spec) resolves this honestly by annotating the spec with current behavior + deferral pointer, rather than deleting the ambition or silently building features without product direction.

## Goals / Non-Goals

**Goals:**
- Every spec statement is either true today or explicitly marked Deferred with a tracking pointer (beads id / future change).

**Non-Goals:**
- No feature implementation; no requirement deletions (ambitions stay, marked).

## Decisions

- Annotation pattern: append a `**Deferred**：…` paragraph (Chinese, matching the specs' bilingual style) stating current behavior and the tracking item.
- acp-session-mapping H2 contradiction resolved in favor of the implemented+documented routing-id fallback (the "MUST NOT guess" aspiration predated the mapping change's own legacy-fallback clause).

## Risks / Trade-offs

- A deferred marker could be mistaken for acceptance of a permanent gap; mitigated by pointing each marker at a live tracking item (beads).
