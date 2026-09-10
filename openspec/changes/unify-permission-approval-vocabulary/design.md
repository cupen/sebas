## Context

The cross-driver permission vocabulary drifted: `agent-driver` promises a uniform `allow_once/allow_session/deny/escalate` set, but `escalate` exists only on the native kernel; the ACP `Decision` enum has three variants and the webui silently demotes `escalate` → `allow_once` (`session_backend.rs`). The word "escalate" also names the ACP hang-detection kill ladder, and the glossary did not disambiguate. `permission-flow` claims a 3-decision Feishu surface and a full Claude round-trip but overlaps `agent-driver`'s cross-driver routing.

## Goals / Non-Goals

**Goals:**
- Make the permission decision vocabulary explicit and honest: `escalate` is native-only; ACP demotes to `allow_once` (with a log).
- Disambiguate "escalate" (approval decision vs kill ladder) and "ACP" (dead private bridge vs live standard).
- Re-scope `permission-flow` to the Feishu rendering + hook-path allowlist it actually owns.

**Non-Goals:**
- No behavior change (the `escalate`→`allow_once` demotion is ratified, not redesigned).
- No rewrite of the hook park/correlation mechanism.
- No new glossary mechanism; reuse the existing 三义消解 block.

## Decisions

- Keep `escalate` in the vocabulary but scope it to native; specify the ACP demotion as a SHALL (log the downgrade) rather than leaving it implicit.
- Rename the ACP hang sequence to "kill ladder" in `acp-driver` text so "escalate" is unambiguous.
- Record terminology disambiguation in the glossary (escalate, ACP), keeping `permission-flow`'s hook mechanics intact.

## Risks / Trade-offs

- Specifying the demotion makes the silent fallback visible; a future change may instead add a real ACP escalate mapping (out of scope).
- Re-scoping `permission-flow` reduces its claimed surface; acceptable because `agent-driver` already owns cross-driver routing.
